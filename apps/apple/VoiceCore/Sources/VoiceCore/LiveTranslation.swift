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
    /// `epoch`: Epoche des Aufrufers (Sprung/Wechsel, siehe `enter`).
    public func prefetch(_ units: [LiveUnit], epoch: Int? = nil) {
        guard enter(epoch) else { return }
        let missing = units.filter { cache.peek(key($0)) == nil && inflight[key($0)] == nil }
        guard let first = missing.first else { return }
        start([first])
        for batch in LiveTranslation.batches(Array(missing.dropFirst())) { start(batch) }
    }

    /// Übersetzung einer Einheit; wartet auf einen laufenden Stapel oder startet einen eigenen.
    /// Wirft `CancellationError`, wenn die Generation inzwischen gewechselt hat.
    public func text(_ unit: LiveUnit, epoch: Int? = nil) async throws -> String {
        guard enter(epoch) else { throw CancellationError() }
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
    public func cancelPending() { invalidate(to: generation + 1) }

    /// Bindet Aufrufe an die Epoche des Aufrufers (Spez §4.1). Die erste Anfrage einer neueren Epoche verwirft zuerst
    /// alle laufenden Stapel und bestellt dann neu – im Actor und damit geordnet, ohne nebenläufiges
    /// `cancelPending()`. Anfragen einer älteren Epoche bestellen nichts mehr.
    private func enter(_ epoch: Int?) -> Bool {
        guard let epoch else { return true }
        if epoch > generation { invalidate(to: epoch) }
        return epoch == generation
    }

    private func invalidate(to newGeneration: Int) {
        generation = newGeneration
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

/// Anzeige eines Absatzes im Live-Modus (Spez §6): übersetzte Sätze in der Zielsprache, noch nicht übersetzte
/// im Original. Bereiche in UTF-16 des zusammengesetzten Texts, je Quellsatz einer.
public struct LiveParagraph: Equatable, Sendable {
    public let text: String
    /// Bereich jedes Quellsatzes im angezeigten Text (gleiche Reihenfolge wie die Satzbereiche der Quelle).
    public let ranges: [NSRange]
    /// Sätze, die noch im Original stehen (gedämpft anzeigen).
    public let pending: Set<Int>

    /// Setzt den Absatz aus Originalsätzen und vorhandenen Übersetzungen (`translated[satz]`) zusammen.
    /// Zwischen Sätzen steht ein Leerzeichen; Text vor dem ersten Satz (Einzug, Anführung) bleibt erhalten.
    public static func compose(original: String, sentences: [NSRange], translated: [Int: String]) -> LiveParagraph {
        let all = original as NSString
        var text = sentences.first.map { all.substring(to: $0.location) } ?? original
        var ranges: [NSRange] = []
        var pending: Set<Int> = []
        for (i, range) in sentences.enumerated() {
            if i > 0 { text += " " }
            let piece: String
            if let t = translated[i]?.trimmingCharacters(in: .whitespacesAndNewlines), !t.isEmpty { piece = t }
            else { piece = all.substring(with: range); pending.insert(i) }
            ranges.append(NSRange(location: text.utf16.count, length: piece.utf16.count))
            text += piece
        }
        return LiveParagraph(text: text, ranges: ranges, pending: pending)
    }

    /// Bereich des Quellsatzes Nr. `index` im angezeigten Text.
    public func range(forSentence index: Int) -> NSRange? { ranges.indices.contains(index) ? ranges[index] : nil }
}

/// Sprachen, die der Sprach-Chip anbietet (Spez §5). Ob Apple sie übersetzen kann, prüft das iPhone zur Laufzeit.
public enum LiveLanguages {
    public static let offered: [(code: String, name: String)] = [
        ("de", "Deutsch"), ("en", "Englisch"), ("fr", "Französisch"), ("es", "Spanisch"), ("it", "Italienisch"),
        ("pt", "Portugiesisch"), ("nl", "Niederländisch"), ("pl", "Polnisch"), ("tr", "Türkisch"), ("uk", "Ukrainisch"),
        ("ru", "Russisch"), ("zh-hans", "Chinesisch (vereinfacht)"), ("zh-hant", "Chinesisch (traditionell)"),
        ("ja", "Japanisch"), ("ko", "Koreanisch"), ("ar", "Arabisch"), ("hi", "Hindi"), ("id", "Indonesisch"),
        ("th", "Thailändisch"), ("vi", "Vietnamesisch"),
    ]

    public static func name(_ code: String) -> String {
        let c = LiveTranslation.base(code)
        return offered.first { $0.code == c }?.name ?? c.uppercased()
    }

    /// Kurzform für den Chip („EN → DE“, „ZH-HANS“ → „ZH“).
    public static func short(_ code: String) -> String {
        String(LiveTranslation.base(code).split(separator: "-").first ?? "").uppercased()
    }
}

/// Spiegel der fertigen Übersetzungen für Anzeige und Sprechen auf dem Main Actor (Spez §4, §4.1):
/// begrenzt wie der Pipeline-Speicher und mit Epoche, damit Ergebnisse aus der Zeit vor einem Sprung oder
/// Sprachwechsel nicht mehr hineinkommen.
public struct LiveMirror: Sendable {
    public struct Slot: Hashable, Sendable { public let paragraph: Int; public let sentence: Int }
    public let limit: Int
    public private(set) var epoch = 0
    private var texts: [Int: [Int: String]] = [:]
    private var order: [Slot] = []

    public init(limit: Int = LiveTranslation.cacheLimit) { self.limit = max(1, limit) }

    public var count: Int { order.count }

    /// Sprung: neue Epoche, fertige Übersetzungen bleiben (sie gehören zur selben Sprache).
    public mutating func advance() { epoch += 1 }

    /// Sprachwechsel oder neues Buch: alles verwerfen, neue Epoche.
    public mutating func reset() { texts = [:]; order = []; epoch += 1 }

    /// Nur Ergebnisse der laufenden Epoche; ältere werden verworfen. Ältester Eintrag fällt über der Grenze heraus.
    @discardableResult
    public mutating func store(_ text: String, paragraph: Int, sentence: Int, epoch e: Int) -> Bool {
        guard e == epoch else { return false }
        let slot = Slot(paragraph: paragraph, sentence: sentence)
        if texts[paragraph]?[sentence] != nil, let i = order.firstIndex(of: slot) { order.remove(at: i) }
        texts[paragraph, default: [:]][sentence] = text
        order.append(slot)
        while order.count > limit {
            let old = order.removeFirst()
            texts[old.paragraph]?[old.sentence] = nil
            if texts[old.paragraph]?.isEmpty == true { texts[old.paragraph] = nil }
        }
        return true
    }

    /// Lesen zählt als Nutzung (LRU): der Satz rückt ans Ende der Verdrängungsreihe.
    public mutating func text(paragraph: Int, sentence: Int) -> String? {
        guard let value = texts[paragraph]?[sentence] else { return nil }
        let slot = Slot(paragraph: paragraph, sentence: sentence)
        if let i = order.firstIndex(of: slot) { order.remove(at: i); order.append(slot) }
        return value
    }
    /// Nur nachsehen, ohne die Reihenfolge zu ändern.
    public func peek(paragraph: Int, sentence: Int) -> String? { texts[paragraph]?[sentence] }
    public func paragraph(_ index: Int) -> [Int: String] { texts[index] ?? [:] }
}

/// Wann ein Sprachwechsel greift (Spez §5).
public enum LiveSwitchTiming: Equatable, Sendable {
    /// Pausiert oder still: sofort am aktuellen Satz.
    case immediate
    /// Ton läuft: komplett (Ziel, Pipeline, Stimme) am Ende des hörbaren Satzes.
    case atSentenceEnd
    /// Noch kein Ton (wird vorbereitet): sofort umstellen und am aktuellen Satz neu ansetzen.
    case restart

    public static func decide(busy: Bool, preparing: Bool) -> LiveSwitchTiming {
        if preparing { return .restart }
        return busy ? .atSentenceEnd : .immediate
    }
}

/// Gewähltes Sprachpaar einer Live-Sitzung.
public struct LiveConfig: Hashable, Sendable {
    public let source: String
    public let target: String
    public init(source: String, target: String) { self.source = LiveTranslation.base(source); self.target = LiveTranslation.base(target) }
}

/// Sprachwechsel als reine Zustandsmaschine (Spez §5): Ziel, Quelle, Übersetzer und Stimme wechseln immer
/// gemeinsam. Läuft der Ton, wird nur vorgemerkt; der Wechsel greift komplett am Ende des hörbaren Satzes.
/// Verglichen wird das ganze Paar, damit eine korrigierte Quellsprache den Übersetzer auch bei gleicher
/// Zielsprache neu aufbaut.
public struct LiveSwitch: Equatable, Sendable {
    public enum Effect: Equatable, Sendable {
        case none
        /// Vorgemerkt, greift am Satzende.
        case scheduled
        /// Jetzt umstellen (`nil`: Original). `restart`: am aktuellen Satz neu ansetzen, sonst nur vorbereiten.
        case apply(LiveConfig?, restart: Bool)
    }

    public private(set) var source: String?
    public private(set) var active: LiveConfig?
    /// `.some(nil)`: zurück zum Original vorgemerkt.
    public private(set) var pending: LiveConfig??

    public init(source: String? = nil) { self.source = source.map(LiveTranslation.base) }

    /// Gewollte Zielsprache: vorgemerkt, sonst aktiv.
    public var wantedTarget: String? {
        if let pending { return pending?.target }
        return active?.target
    }

    public mutating func select(_ target: String?, busy: Bool, preparing: Bool) -> Effect {
        route(config(target), busy: busy, preparing: preparing)
    }

    /// Bestätigte oder korrigierte Buchsprache; die gewollte Zielsprache bleibt.
    public mutating func changeSource(_ newSource: String?, busy: Bool, preparing: Bool) -> Effect {
        let target = wantedTarget
        source = newSource.map(LiveTranslation.base)
        return route(config(target), busy: busy, preparing: preparing)
    }

    /// Hörbarer Satz zu Ende (und es geht weiter): ein vorgemerkter Wechsel greift jetzt komplett.
    public mutating func sentenceEnded() -> Effect { take(restart: true) }

    /// Pause, Unterbrechung oder Ende ohne Ton: ein vorgemerkter Wechsel greift sofort, ohne Neustart.
    public mutating func stopped() -> Effect { take(restart: false) }

    /// Das Gerät kann das Paar nicht übersetzen: zurück zum Original.
    public mutating func fallBack() { active = nil; pending = nil }

    private func config(_ target: String?) -> LiveConfig? {
        guard let target, let source else { return nil }
        let wanted = LiveConfig(source: source, target: target)
        return wanted.target == wanted.source ? nil : wanted
    }

    private mutating func route(_ desired: LiveConfig?, busy: Bool, preparing: Bool) -> Effect {
        let timing = LiveSwitchTiming.decide(busy: busy, preparing: preparing)
        if timing == .atSentenceEnd {
            pending = desired == active ? nil : .some(desired)
            return pending == nil ? .none : .scheduled
        }
        pending = nil
        guard desired != active else { return .none }
        active = desired
        return .apply(desired, restart: timing == .restart)
    }

    private mutating func take(restart: Bool) -> Effect {
        guard let next = pending else { return .none }
        pending = nil
        guard next != active else { return .none }
        active = next
        return .apply(next, restart: restart)
    }
}

/// Positionsvertrag beim Vorlesen (Spez §4.1): Der Stand zeigt auf den Beginn des nächsten ungelesenen Satzes und
/// rückt nur vor, wenn ein Satz zu Ende gehört ist. Vorlauf, Pause, Unterbrechung und Unterlauf ändern ihn nie;
/// Fortsetzen beginnt am hörbaren Satz.
public struct LiveProgress: Equatable, Sendable {
    /// Gespeicherter Stand (UTF-16 im Gesamttext).
    public private(set) var saved: Int
    /// Beginn des hörbaren Satzes; dort setzt Fortsetzen an.
    public private(set) var audible: Int
    /// Zuletzt als gehört gezählter Bereich.
    public private(set) var heard: Range<Int>?

    public init(at offset: Int) { saved = offset; audible = offset }

    /// Satz zu Ende gehört. `next`: Beginn des nächsten Satzes und ob er im selben Absatz liegt; `nil` am Buchende.
    public mutating func finished(sentenceStart: Int, paragraphEnd: Int, next: (start: Int, sameParagraph: Bool)?) {
        let to = next?.start ?? paragraphEnd
        heard = sentenceStart..<max(sentenceStart, next?.sameParagraph == true ? to : paragraphEnd)
        saved = to
        audible = to
    }

    /// Angesprungen: Stand an die neue Stelle, das Dazwischen zählt nicht als gehört.
    public mutating func jumped(to offset: Int) { saved = offset; audible = offset; heard = nil }

    /// Pause oder Unterbrechung mitten im Satz: nichts ändert sich, Fortsetzen wiederholt den Satz.
    public func resumeOffset() -> Int { audible }
}

/// Warten auf einen Satz mit Unterlaufregel (Spez §4): nach `hint` Hinweis „Übersetzung lädt …“, nach `stall`
/// Pause mit Grund. Ein Übersetzer, der Abbruch ignoriert, hält das Warten nicht auf.
public enum LiveWait {
    public static let hintNanos: UInt64 = 1_500_000_000
    public static let stallNanos: UInt64 = 8_000_000_000

    public enum Outcome: Sendable {
        case text(String)
        case failed(any Error)
        case cancelled
        case timedOut
    }

    public static func run(hint: UInt64 = hintNanos, stall: UInt64 = stallNanos,
                           onHint: @escaping @Sendable () async -> Void,
                           work: @escaping @Sendable () async throws -> String) async -> Outcome {
        await withCheckedContinuation { (cont: CheckedContinuation<Outcome, Never>) in
            let once = LiveOnce(cont)
            let job = Task {
                do { once.finish(.text(try await work())) }
                catch is CancellationError { once.finish(.cancelled) }
                catch { once.finish(.failed(error)) }
            }
            once.add(Task {
                try? await Task.sleep(nanoseconds: hint)
                if !Task.isCancelled { await onHint() }
            })
            once.add(Task {
                try? await Task.sleep(nanoseconds: stall)
                guard !Task.isCancelled else { return }
                if once.finish(.timedOut) { job.cancel() }
            })
        }
    }
}

/// Liefert genau ein Ergebnis und stoppt danach die Zeitgeber.
private final class LiveOnce: @unchecked Sendable {
    private let lock = NSLock()
    private var cont: CheckedContinuation<LiveWait.Outcome, Never>?
    private var timers: [Task<Void, Never>] = []

    init(_ cont: CheckedContinuation<LiveWait.Outcome, Never>) { self.cont = cont }

    func add(_ timer: Task<Void, Never>) {
        lock.lock()
        let done = cont == nil
        if !done { timers.append(timer) }
        lock.unlock()
        if done { timer.cancel() }
    }

    @discardableResult
    func finish(_ value: LiveWait.Outcome) -> Bool {
        lock.lock()
        let c = cont
        cont = nil
        let pending = timers
        timers = []
        lock.unlock()
        guard let c else { return false }
        pending.forEach { $0.cancel() }
        c.resume(returning: value)
        return true
    }
}
