import Foundation

// Live-Übersetzung beim Vorlesen (lva#358, Spez 2026-11-06-live-uebersetzung.md §3–§4).
// Reiner Kern ohne Apple-Framework: Einheiten bilden, Vorlauf planen, Ergebnisse im
// Arbeitsspeicher halten. Der eigentliche Übersetzer steckt hinter `LiveTranslator`.

/// Übersetzt eine Liste kurzer Texte in derselben Reihenfolge. Geräteintern (Apple) oder später über das Relay.
public protocol LiveTranslator: Sendable {
    func translate(_ texts: [String], from source: String, to target: String) async throws -> [String]
}

/// Vorgelesene Einheit: ein Satz. Bereich in UTF-16 relativ zum Absatz.
public struct LiveUnit: Hashable, Sendable {
    public let paragraph: Int
    public let range: NSRange
    public let text: String
    /// Erster und letzter Satz (Index in `LibraryText.sentences`), die zur Einheit gehören.
    public let sentences: ClosedRange<Int>

    public init(paragraph: Int, range: NSRange, text: String, sentences: ClosedRange<Int>) {
        self.paragraph = paragraph; self.range = range; self.text = text; self.sentences = sentences
    }
}

/// Fehler des Live-Weges; die Wiedergabe zeigt sie an der Stelle, liest nie das Original mit der Zielstimme.
public enum LiveTranslationError: Error, Equatable {
    case countMismatch
    case unusable
}

public enum LiveTranslation {
    public static let minAheadUnits = 6
    public static let minAheadSeconds = 30.0
    public static let maxAheadUnits = 40
    public static let cacheLimit = 2000
    /// Höchstens so viele Sätze je Übersetzungsaufruf; lange Absätze werden geteilt.
    public static let maxBatch = 12
    /// Erwartete Sprechgeschwindigkeit bei Tempo 1 (Zeichen je Sekunde), nur für die Vorlaufschätzung.
    public static let charsPerSecond = 15.0

    /// Einheiten eines Absatzes: ein Satz je Einheit (Codex F1, kein Verbinden kurzer Sätze in V1).
    public static func units(paragraph: Int, text: String, sentences: [NSRange]) -> [LiveUnit] {
        let all = text as NSString
        return sentences.enumerated().map { i, range in
            LiveUnit(paragraph: paragraph, range: range, text: all.substring(with: range), sentences: i...i)
        }
    }

    /// Einheit, die den Satz `sentence` enthält (sonst die letzte davor).
    public static func unitIndex(_ units: [LiveUnit], sentence: Int) -> Int {
        units.lastIndex { $0.sentences.lowerBound <= sentence } ?? 0
    }

    /// Geschätzte Sprechzeit in Sekunden.
    public static func seconds(_ text: String, rate: Double = 1) -> Double {
        Double(text.utf16.count) / (charsPerSecond * max(rate, 0.1))
    }

    /// Vorlauffenster ab `unit` im Absatz `paragraph`: so viele Einheiten, bis Mindestzahl und Mindestzeit
    /// erreicht sind; die Höchstgrenze `maxAheadUnits` hat Vorrang. `units` liefert die Einheiten eines Absatzes.
    public static func window(paragraph: Int, unit: Int, paragraphCount: Int, rate: Double = 1,
                              units: (Int) -> [LiveUnit]) -> [LiveUnit] {
        var out: [LiveUnit] = []
        var secs = 0.0
        var p = paragraph, u = unit
        while p < paragraphCount, out.count < maxAheadUnits {
            let list = units(p)
            if u < list.count {
                out.append(list[u]); secs += seconds(list[u].text, rate: rate); u += 1
                if out.count >= minAheadUnits, secs >= minAheadSeconds { break }
            } else { p += 1; u = 0 }
        }
        return out
    }

    /// Fehlende Einheiten in Stapel je Absatz, in Lesereihenfolge.
    public static func batches(_ units: [LiveUnit]) -> [[LiveUnit]] {
        var out: [[LiveUnit]] = []
        for unit in units {
            if let last = out.last?.last, last.paragraph == unit.paragraph, out[out.count - 1].count < maxBatch {
                out[out.count - 1].append(unit)
            }
            else { out.append([unit]) }
        }
        return out
    }

    /// Antwort brauchbar? Nur eine leere Antwort ist ein Fehler.
    public static func usable(_ translated: String) -> Bool {
        !translated.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Unveränderte Rückgabe eines längeren Satzes zwischen verschiedenen Sprachen: einmal neu versuchen,
    /// danach übernehmen (Namen, Zahlen, Zitate dürfen gleich bleiben).
    public static func worthRetry(source: String, translated: String, from: String, to: String) -> Bool {
        guard usable(translated) else { return true }
        guard base(from) != base(to) else { return false }
        let s = source.trimmingCharacters(in: .whitespacesAndNewlines)
        let t = translated.trimmingCharacters(in: .whitespacesAndNewlines)
        return s.split(whereSeparator: { $0.isWhitespace }).count >= 4 && s.caseInsensitiveCompare(t) == .orderedSame
    }

    /// Sprachkennung normalisiert wie im Erzeugungsschlüssel (Spez §7.3): klein, Sprache und Schrift,
    /// ohne Region („en-GB“ → „en“, „zh_Hans_CN“ → „zh-hans“, „sr-Latn“ → „sr-latn“).
    public static func base(_ lang: String) -> String {
        let parts = lang.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" }).map(String.init)
        guard let language = parts.first else { return "" }
        if parts.count > 1, parts[1].count == 4, parts[1].allSatisfy({ $0.isLetter }) { return language + "-" + parts[1] }
        return language
    }
}

/// Schlüssel eines übersetzten Stücks: Buch, Quellfassung, Zielsprache, Lage im Absatz.
public struct LiveKey: Hashable, Sendable {
    public let book: String
    public let sourceHash: String
    public let target: String
    public let paragraph: Int
    public let location: Int
    public let length: Int

    public init(book: String, sourceHash: String, target: String, unit: LiveUnit) {
        self.book = book; self.sourceHash = sourceHash; self.target = LiveTranslation.base(target)
        paragraph = unit.paragraph; location = unit.range.location; length = unit.range.length
    }
}

/// Übersetzungen nur im Arbeitsspeicher, älteste zuerst verdrängt.
public struct LiveCache: Sendable {
    public let limit: Int
    private var store: [LiveKey: String] = [:]
    private var order: [LiveKey] = []

    public init(limit: Int = LiveTranslation.cacheLimit) { self.limit = max(1, limit) }

    public var count: Int { store.count }

    public mutating func get(_ key: LiveKey) -> String? {
        guard let value = store[key] else { return nil }
        touch(key)
        return value
    }

    public func peek(_ key: LiveKey) -> String? { store[key] }

    public mutating func put(_ key: LiveKey, _ value: String) {
        if store.updateValue(value, forKey: key) != nil { touch(key); return }
        order.append(key)
        while order.count > limit { store[order.removeFirst()] = nil }
    }

    private mutating func touch(_ key: LiveKey) {
        if let i = order.firstIndex(of: key) { order.remove(at: i) }
        order.append(key)
    }
}

/// Fließband für eine Sitzung (ein Buch, eine Quellfassung, eine Zielsprache). Übersetzt Stapel je Absatz,
/// teilt laufende Stapel zwischen Wartenden und zählt die übersetzten Zeichen (Abnahme: nur Gelesenes plus Vorlauf).
public actor LivePipeline {
    public let book: String
    public let sourceHash: String
    public let source: String
    public let target: String
    private let translator: any LiveTranslator
    private var cache: LiveCache
    private struct Flight { let id: Int; let task: Task<[LiveKey: String], Error> }
    private var inflight: [LiveKey: Flight] = [:]
    private var nextFlight = 0
    private var generation = 0
    /// Übersetzte Zeichen und Aufrufe, Wiederholungen eingeschlossen.
    public private(set) var translatedChars = 0
    public private(set) var calls = 0

    public init(translator: any LiveTranslator, book: String, sourceHash: String, source: String, target: String,
                cacheLimit: Int = LiveTranslation.cacheLimit) {
        self.translator = translator; self.book = book; self.sourceHash = sourceHash
        self.source = source; self.target = target; cache = LiveCache(limit: cacheLimit)
    }

    public func key(_ unit: LiveUnit) -> LiveKey { LiveKey(book: book, sourceHash: sourceHash, target: target, unit: unit) }

    /// Schon übersetzt? Für die Anzeige (noch nicht übersetzte Einheiten erscheinen gedämpft im Original).
    public func ready(_ unit: LiveUnit) -> String? { cache.peek(key(unit)) }

    /// Wiedergabegeneration (Spez §4.1): steigt bei Sprung und Wechsel; ältere Ergebnisse gelangen weder in den
    /// Speicher noch zu Wartenden.
    public var currentGeneration: Int { generation }

    /// Fenster anstoßen. Die erste fehlende Einheit läuft vorrangig allein (Spez §4 Start), der Rest als
    /// Absatzstapel. Kehrt sofort zurück.
    public func prefetch(_ units: [LiveUnit]) {
        let missing = units.filter { cache.peek(key($0)) == nil && inflight[key($0)] == nil }
        guard let first = missing.first else { return }
        start([first])
        for batch in LiveTranslation.batches(Array(missing.dropFirst())) { start(batch) }
    }

    /// Übersetzung einer Einheit; wartet auf einen laufenden Stapel oder startet einen eigenen.
    /// Wirft `CancellationError`, wenn die Generation inzwischen gewechselt hat.
    public func text(_ unit: LiveUnit) async throws -> String {
        let gen = generation
        let k = key(unit)
        if let hit = cache.get(k) { return hit }
        let task = inflight[k]?.task ?? start([unit])
        let result: [LiveKey: String]
        do { result = try await task.value }
        catch {
            // Auch im Fehlerpfad: ein Wechsel der Generation hat Vorrang vor jedem späten Fehler.
            guard gen == generation else { throw CancellationError() }
            throw error
        }
        guard gen == generation else { throw CancellationError() }
        guard let value = result[k] else { throw LiveTranslationError.unusable }
        return value
    }

    /// Sprung oder Sprachwechsel: laufende Stapel verwerfen. Fertige Übersetzungen bleiben im Speicher.
    public func cancelPending() {
        generation += 1
        for flight in inflight.values { flight.task.cancel() }
        inflight.removeAll()
    }

    @discardableResult
    private func start(_ batch: [LiveUnit]) -> Task<[LiveKey: String], Error> {
        let keys = batch.map(key)
        let texts = batch.map(\.text)
        let gen = generation
        nextFlight += 1
        let id = nextFlight
        let task = Task { () throws -> [LiveKey: String] in
            defer { self.release(keys, id: id) }
            let out = try await self.translateWithRetry(texts)
            try Task.checkCancellation()
            var result: [LiveKey: String] = [:]
            for (i, k) in keys.enumerated() where LiveTranslation.usable(out[i]) { result[k] = out[i] }
            if gen == self.generation { for (k, v) in result { self.cache.put(k, v) } }
            return result
        }
        for k in keys { inflight[k] = Flight(id: id, task: task) }
        return task
    }

    /// Ein Aufruf mit Zählung und Prüfung der Antwortanzahl.
    private func call(_ texts: [String]) async throws -> [String] {
        calls += 1
        translatedChars += texts.reduce(0) { $0 + $1.utf16.count }
        let out = try await translator.translate(texts, from: source, to: target)
        guard out.count == texts.count else { throw LiveTranslationError.countMismatch }
        return out
    }

    /// Spez §4 mit gemeinsamem Budget von höchstens zwei Aufrufen:
    /// - wirft der erste Aufruf (auch falsche Anzahl), ist der zweite die einzige Wiederholung; dessen Antwort gilt
    ///   ohne weitere Nachfrage (leere Sätze bleiben unbrauchbar);
    /// - sonst fragt der zweite Aufruf leere oder auffällig unveränderte Sätze nach. Fehler dieser Nachfrage werden
    ///   weitergereicht (Wiedergabe pausiert); eine unveränderte Antwort gilt nur, wenn die Nachfrage gelang.
    private func translateWithRetry(_ texts: [String]) async throws -> [String] {
        var out: [String]
        do { out = try await call(texts) }
        catch is CancellationError { throw CancellationError() }
        catch {
            try Task.checkCancellation()
            return try await call(texts)
        }
        let bad = out.indices.filter { LiveTranslation.worthRetry(source: texts[$0], translated: out[$0], from: source, to: target) }
        guard !bad.isEmpty else { return out }
        try Task.checkCancellation()
        let again = try await call(bad.map { texts[$0] })
        for (i, value) in zip(bad, again) { out[i] = value }
        return out
    }

    /// Auf jedem Abschlussweg aufräumen, aber nur die eigenen Einträge (neuere Aufträge bleiben).
    private func release(_ keys: [LiveKey], id: Int) {
        for k in keys where inflight[k]?.id == id { inflight[k] = nil }
    }
}
