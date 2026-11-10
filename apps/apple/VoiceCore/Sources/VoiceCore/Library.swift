import Foundation
import NaturalLanguage

// Bibliothek (Epic #210): dasselbe Datenmodell wie die Desktop-Bibliothek
// (`apps/local-voice/src-tauri/src/library.rs`, `src/lib/library/*.ts`), damit
// Einträge und Fortschritt zwischen PC und iPhone ohne Umrechnung wandern.
//
// Zeichenstellen sind UTF-16-Einheiten wie in JavaScript (`String.length`),
// nicht Swift-Characters: so meinen `chars` in Zeitachse und Fortschritt auf
// beiden Seiten dieselbe Stelle.

public struct LibraryTextVersion: Codable, Equatable, Sendable {
    public var pageId: String
    /// `document` oder `transcript`.
    public var origin: String
    public var sourceName: String?
    public init(pageId: String, origin: String, sourceName: String? = nil) {
        self.pageId = pageId; self.origin = origin; self.sourceName = sourceName
    }
    enum CodingKeys: String, CodingKey { case pageId = "page_id", origin, sourceName = "source_name" }
}

public struct LibraryAudioTrack: Codable, Equatable, Sendable {
    public var file: String
    public var title: String
    public var durationMs: Double?
    public var transcriptJob: String?
    public init(file: String, title: String, durationMs: Double? = nil, transcriptJob: String? = nil) {
        self.file = file; self.title = title; self.durationMs = durationMs; self.transcriptJob = transcriptJob
    }
    enum CodingKeys: String, CodingKey { case file, title, durationMs = "duration_ms", transcriptJob = "transcript_job" }
}

/// Stützpunkt der Zeitachse: ab Zeichenstelle `chars` ist das Hörbuch bei `ms`.
public struct LibrarySyncPoint: Codable, Equatable, Sendable {
    public var chars: Double
    public var ms: Double
    public init(chars: Double, ms: Double) { self.chars = chars; self.ms = ms }
}

public struct LibraryAudioVersion: Codable, Equatable, Sendable {
    public var file: String
    public var durationMs: Double?
    public var narrator: String?
    public var transcriptJob: String?
    public var tracks: [LibraryAudioTrack]
    public var sync: [LibrarySyncPoint]
    public init(file: String, durationMs: Double? = nil, narrator: String? = nil, transcriptJob: String? = nil,
                tracks: [LibraryAudioTrack] = [], sync: [LibrarySyncPoint] = []) {
        self.file = file; self.durationMs = durationMs; self.narrator = narrator
        self.transcriptJob = transcriptJob; self.tracks = tracks; self.sync = sync
    }
    enum CodingKeys: String, CodingKey {
        case file, durationMs = "duration_ms", narrator, transcriptJob = "transcript_job", tracks, sync
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        file = try c.decodeIfPresent(String.self, forKey: .file) ?? ""
        durationMs = try c.decodeIfPresent(Double.self, forKey: .durationMs)
        narrator = try c.decodeIfPresent(String.self, forKey: .narrator)
        transcriptJob = try c.decodeIfPresent(String.self, forKey: .transcriptJob)
        tracks = try c.decodeIfPresent([LibraryAudioTrack].self, forKey: .tracks) ?? []
        sync = try c.decodeIfPresent([LibrarySyncPoint].self, forKey: .sync) ?? []
    }

    /// Teile in Hörreihenfolge; Altbestand ohne `tracks` hat genau einen Teil.
    public var allTracks: [LibraryAudioTrack] {
        if !tracks.isEmpty { return tracks }
        if file.isEmpty { return [] }
        let stem = (file as NSString).deletingPathExtension
        return [LibraryAudioTrack(file: file, title: stem, durationMs: durationMs, transcriptJob: transcriptJob)]
    }

    /// Gesamtdauer, sofern jeder Teil eine Dauer kennt.
    public var totalDurationMs: Double? {
        let parts = allTracks
        if parts.isEmpty { return durationMs }
        var sum = 0.0
        for part in parts { guard let d = part.durationMs else { return durationMs }; sum += d }
        return sum
    }
}

public struct LibraryBook: Codable, Equatable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var subtitle: String?
    public var authors: [String]
    public var language: String?
    public var isbn: String?
    public var publisher: String?
    public var publishDate: String?
    public var pages: Int?
    public var description: String?
    public var metadataSource: String?
    public var coverSource: String
    public var text: LibraryTextVersion?
    public var audio: LibraryAudioVersion?
    public var addedMs: Double
    public var finished: Bool
    /// Favorit (Desktop lva#222); fehlt in älteren buch.json.
    public var favorite: Bool = false
    /// Sprachfassungen (lva#344, Schema 2). Das iPhone reicht sie unverändert
    /// weiter; `text`/`audio`/`language` bleiben der Spiegel der Hauptfassung.
    public var schema: Int?
    public var workKey: String?
    public var editions: PassthroughJSON?
    public var primary: PassthroughJSON?
    public var editionTombstones: PassthroughJSON?
    public var reading: PassthroughJSON?
    public var alignments: PassthroughJSON?
    /// Nur zur Laufzeit, nie gespeichert und nie im Abgleich (kein Eintrag in `CodingKeys`): Ist es gesetzt, ist
    /// dieses Buch die Sicht auf eine Sprachfassung. `title`, `subtitle`, `language`, `text` und `audio` sind dann die
    /// der Fassung, alles Übrige das Buch selbst (`PhoneLibrary.editionView`).
    public var editionId: String? = nil

    public init(id: String, title: String, authors: [String] = [], coverSource: String = "generated",
                text: LibraryTextVersion? = nil, audio: LibraryAudioVersion? = nil, addedMs: Double, finished: Bool = false) {
        self.id = id; self.title = title; self.authors = authors; self.coverSource = coverSource
        self.text = text; self.audio = audio; self.addedMs = addedMs; self.finished = finished
    }

    enum CodingKeys: String, CodingKey {
        case id, title, subtitle, authors, language, isbn, publisher, publishDate = "publish_date", pages, description
        case metadataSource = "metadata_source", coverSource = "cover_source", text, audio, addedMs = "added_ms", finished, favorite
        case schema, workKey = "work_key", editions, primary, editionTombstones = "edition_tombstones", reading, alignments
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        title = try c.decode(String.self, forKey: .title)
        subtitle = try c.decodeIfPresent(String.self, forKey: .subtitle)
        authors = try c.decodeIfPresent([String].self, forKey: .authors) ?? []
        language = try c.decodeIfPresent(String.self, forKey: .language)
        isbn = try c.decodeIfPresent(String.self, forKey: .isbn)
        publisher = try c.decodeIfPresent(String.self, forKey: .publisher)
        publishDate = try c.decodeIfPresent(String.self, forKey: .publishDate)
        pages = try c.decodeIfPresent(Int.self, forKey: .pages)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        metadataSource = try c.decodeIfPresent(String.self, forKey: .metadataSource)
        coverSource = try c.decodeIfPresent(String.self, forKey: .coverSource) ?? "generated"
        text = try c.decodeIfPresent(LibraryTextVersion.self, forKey: .text)
        audio = try c.decodeIfPresent(LibraryAudioVersion.self, forKey: .audio)
        addedMs = try c.decodeIfPresent(Double.self, forKey: .addedMs) ?? 0
        finished = try c.decodeIfPresent(Bool.self, forKey: .finished) ?? false
        favorite = try c.decodeIfPresent(Bool.self, forKey: .favorite) ?? false
        schema = try c.decodeIfPresent(Int.self, forKey: .schema)
        workKey = try c.decodeIfPresent(String.self, forKey: .workKey)
        editions = try c.decodeIfPresent(PassthroughJSON.self, forKey: .editions)
        primary = try c.decodeIfPresent(PassthroughJSON.self, forKey: .primary)
        editionTombstones = try c.decodeIfPresent(PassthroughJSON.self, forKey: .editionTombstones)
        reading = try c.decodeIfPresent(PassthroughJSON.self, forKey: .reading)
        alignments = try c.decodeIfPresent(PassthroughJSON.self, forKey: .alignments)
    }
}

// MARK: - Fortschritt

public enum ProgressUnit: String, Codable, Sendable { case chars, ms }

/// Ein Bereich [von, bis) in Zeichen bzw. Millisekunden.
public typealias ProgressRange = [Double]

/// Wie `ProgressRecord` in `src/lib/library/progress.ts`.
public struct ProgressRecord: Codable, Equatable, Sendable {
    public var offset: Double
    public var anchor: String?
    public var length: Double
    public var heard: [ProgressRange]
    public var read: [ProgressRange]
    public var updated: Double
    public var finished: Bool?
    public var unit: ProgressUnit?
    /// Weitester gelesener oder gehörter Stand in der Einheit des Datensatzes (Buch-Gespräch, Lesegrenze).
    /// Optional und nur steigend: Zurückblättern, Zurückspulen und Zusammenführen senken ihn nie.
    public var frontier: Double?

    public init(offset: Double, anchor: String? = nil, length: Double, heard: [ProgressRange] = [], read: [ProgressRange] = [],
                updated: Double, finished: Bool? = nil, unit: ProgressUnit? = nil, frontier: Double? = nil) {
        self.offset = offset; self.anchor = anchor; self.length = length; self.heard = heard
        self.read = read; self.updated = updated; self.finished = finished; self.unit = unit; self.frontier = frontier
    }
    enum CodingKeys: String, CodingKey { case offset, anchor, length, heard, read, updated, finished, unit, frontier }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        offset = try c.decode(Double.self, forKey: .offset)
        anchor = try c.decodeIfPresent(String.self, forKey: .anchor)
        length = try c.decodeIfPresent(Double.self, forKey: .length) ?? 0
        heard = (try? c.decodeIfPresent([ProgressRange].self, forKey: .heard)) ?? []
        read = (try? c.decodeIfPresent([ProgressRange].self, forKey: .read)) ?? []
        updated = try c.decodeIfPresent(Double.self, forKey: .updated) ?? 0
        finished = try c.decodeIfPresent(Bool.self, forKey: .finished)
        unit = try? c.decodeIfPresent(ProgressUnit.self, forKey: .unit)
        frontier = (try? c.decodeIfPresent(Double.self, forKey: .frontier)).flatMap { $0 }.flatMap { $0.isFinite && $0 >= 0 ? $0 : nil }
    }
}

public enum LibraryProgress {
    /// Größter Sprung, der noch als „durchgehört“ zählt.
    public static func maxStep(_ unit: ProgressUnit) -> Double { unit == .ms ? 120_000 : 4_000 }

    public static func textKey(pageId: String, tab: String = "original") -> String { "text:\(pageId):\(tab)" }
    public static func audioKey(bookId: String) -> String { "audio:\(bookId)" }

    /// Sortiert, überlappende und anstoßende Bereiche vereint, ungültige verworfen.
    public static func mergeRanges(_ ranges: [ProgressRange]) -> [ProgressRange] {
        let sorted = ranges.filter { $0.count == 2 && $0[0].isFinite && $0[1].isFinite && $0[1] > $0[0] }
            .sorted { $0[0] < $1[0] }
        var out: [ProgressRange] = []
        for r in sorted {
            if let last = out.last, r[0] <= last[1] { out[out.count - 1][1] = max(last[1], r[1]) } else { out.append(r) }
        }
        return out
    }

    static func covered(_ ranges: [ProgressRange]) -> Double {
        mergeRanges(ranges).reduce(0) { $0 + ($1[1] - $1[0]) }
    }

    /// Anteil 0…1 wie am Desktop: gehört oder gelesen, „fertig“ = 1.
    public static func fraction(_ rec: ProgressRecord?) -> Double {
        guard let rec else { return 0 }
        if rec.finished == true { return 1 }
        if rec.length <= 0 { return 0 }
        let done = covered(rec.heard + rec.read)
        return max(0, min(1, max(done, rec.offset) / rec.length))
    }

    /// Zwei Fassungen zusammenführen (Gerätewechsel, Sync): Bereiche beider
    /// Seiten bleiben, Stelle und Anker vom neueren, „fertig“ wenn eine Seite
    /// es sagt. Gleichstand bei `updated`: `a` gilt als neuer (wie am Desktop).
    public static func merge(_ a: ProgressRecord, _ b: ProgressRecord) -> ProgressRecord {
        let (older, newer) = a.updated >= b.updated ? (b, a) : (a, b)
        var merged = newer
        if merged.anchor == nil { merged.anchor = older.anchor }
        if merged.unit == nil { merged.unit = older.unit }
        merged.heard = mergeRanges(older.heard + newer.heard)
        merged.read = mergeRanges(older.read + newer.read)
        merged.length = max(newer.length, older.length)
        if older.finished == true || newer.finished == true { merged.finished = true }
        let known = [older.frontier, newer.frontier].compactMap { $0 }
        let ends = rangesEnd(merged)
        merged.frontier = known.isEmpty && ends == 0 ? nil : max(known.max() ?? 0, ends)
        return merged
    }

    /// Zielfassung nach „Rückgängig“ (wie `undoSwitch` am Desktop): Stelle, Anker, Einheit und „fertig“ gehen auf den
    /// Stand VOR dem Wechsel zurück; Bereiche sind die Vereinigung aus damals und seitdem Gehörtem. Gab es vorher
    /// keinen Datensatz, bleibt nur, was seitdem gehört wurde (`nil`, wenn nichts).
    public static func undoingSwitch(current: ProgressRecord?, before: ProgressRecord?, now: Double) -> ProgressRecord? {
        var out: ProgressRecord
        if let before {
            out = before
            if let current {
                out.heard = mergeRanges(before.heard + current.heard)
                out.read = mergeRanges(before.read + current.read)
                out.length = max(before.length, current.length)
                out.frontier = max(before.frontier ?? 0, current.frontier ?? 0, rangesEnd(out))
            }
            out.finished = before.finished == true ? true : nil
        } else {
            guard let current, !(current.heard.isEmpty && current.read.isEmpty) else { return nil }
            out = current
            out.finished = nil
        }
        out.updated = now
        return out
    }

    /// Größtes Ende aller gehörten und gelesenen Bereiche; 0 ohne Bereiche.
    static func rangesEnd(_ rec: ProgressRecord) -> Double {
        (rec.heard + rec.read).reduce(0) { $1.count == 2 && $1[1].isFinite ? max($0, $1[1]) : $0 }
    }

    /// Neuer Wert des Feldes `frontier`: nie unter dem alten, nie unter den Bereichsenden; 0 bleibt 0 (nicht nil).
    /// Ein Altbestand ohne Feld startet bei seinem bisherigen Stand (`reach`), ein frischer Datensatz bei 0:
    /// ein erster Sprung hebt die Grenze nicht.
    static func risenFrontier(prev: ProgressRecord?, rec: ProgressRecord) -> Double {
        let before = prev.map { $0.frontier ?? reach($0) } ?? 0
        return max(before, rangesEnd(rec), rec.finished == true ? rec.length : 0)
    }

    /// Weitester Stand eines Datensatzes in seiner Einheit: Feld und Bereiche; fehlt beides, gilt `offset`
    /// (Altbestand, Plan §3.0). „Fertig“ zählt als ganzer Text.
    public static func reach(_ rec: ProgressRecord) -> Double {
        if rec.finished == true, rec.length > 0 { return rec.length }
        if let frontier = rec.frontier { return max(frontier, rangesEnd(rec)) }
        let ends = rangesEnd(rec)
        if ends > 0 { return ends }
        return max(0, rec.offset.isFinite ? rec.offset : 0)
    }

    /// Lesegrenze des Buch-Gesprächs: `max(gelesen, gehört)` als UTF-16-Offset im Originaltext (Plan §3.0).
    /// Der Hörstand zählt nur mit echter Zeitachse (Abgleich), nie mit der linearen Schätzung. `nil`, wenn
    /// nichts bekannt ist (Kern: Antwort „Ich kenne deine Leseposition noch nicht“).
    public static func frontier(text: ProgressRecord?, audio: ProgressRecord?, map: FollowMap?) -> Int? {
        var best = 0.0
        if let text { best = max(best, reach(text)) }
        if let audio, let map, !map.approximate {
            let ms = reach(audio)
            if ms > 0 { best = max(best, map.chars(atMs: ms)) }
        }
        return best > 0 ? Int(best.rounded(.down)) : nil
    }

    /// Wiedergabe lief von der bisherigen Stelle nach `to`. Kleine Schritte
    /// zählen als gehört bzw. gelesen, große Sprünge nur als neue Stelle.
    public static func recordPosition(_ prev: ProgressRecord?, to: Double, length: Double, anchor: String? = nil,
                                      unit: ProgressUnit = .chars, mode: Mode = .heard, now: Double) -> ProgressRecord {
        let from = prev?.offset ?? 0
        var rec = ProgressRecord(offset: max(0, to), anchor: anchor, length: length > 0 ? length : (prev?.length ?? 0),
                                 heard: prev?.heard ?? [], read: prev?.read ?? [], updated: now,
                                 finished: prev?.finished, unit: unit)
        let step = to - from
        if step > 0 && step <= maxStep(unit) {
            if mode == .read { rec.read = mergeRanges(rec.read + [[from, to]]) } else { rec.heard = mergeRanges(rec.heard + [[from, to]]) }
        }
        if rec.length > 0 && covered(rec.heard + rec.read) >= rec.length * 0.98 { rec.finished = true }
        rec.frontier = risenFrontier(prev: prev, rec: rec)
        return rec
    }

    /// Einen ganzen Bereich als gehört eintragen (z. B. ein fertig vorgelesener
    /// Absatz, der länger ist als `maxStep`); prüft danach „fertig“.
    public static func markHeard(_ rec: ProgressRecord, from: Double, to: Double) -> ProgressRecord {
        var out = rec
        out.heard = mergeRanges(rec.heard + [[from, to]])
        if out.length > 0 && covered(out.heard + out.read) >= out.length * 0.98 { out.finished = true }
        out.frontier = risenFrontier(prev: rec, rec: out)
        return out
    }

    public enum Mode: Sendable { case heard, read }
}

// MARK: - Zeitachse Text ↔ Hörbuch und Absätze

public struct TextParagraph: Equatable, Sendable {
    /// Beginn von `text` im Gesamttext. Bei Überschriften fehlt das Rautezeichen.
    public var textStart: Int { heading ? end - text.utf16.count : start }
    /// Beginn und Ende im Text, in UTF-16-Einheiten.
    public var start: Int
    public var end: Int
    public var text: String
    public var heading: Bool
}

/// Sortierung der Bibliothek, dieselben Werte und Texte wie am Desktop (`library.sort.*`, lva#474).
/// Der Rohwert wird gemerkt (`@AppStorage`) und darf deshalb nicht umbenannt werden.
public enum LibrarySort: String, CaseIterable, Identifiable, Sendable {
    case recent, added, title, author
    public var id: String { rawValue }
    public var label: String {
        switch self {
        case .recent: return "Zuletzt gehört"
        case .added: return "Zuletzt hinzugefügt"
        case .title: return "Titel"
        case .author: return "Autor"
        }
    }
    /// Gemerkter Wert; Unbekanntes (z. B. aus einer neueren Version) fällt auf den Standard zurück.
    public static func stored(_ raw: String) -> LibrarySort { LibrarySort(rawValue: raw) ?? .recent }
}

public enum LibraryText {
    /// Absätze (durch Leerzeilen getrennt) samt Beginn, wie `splitParagraphs` am Desktop.
    public static func paragraphs(_ text: String) -> [TextParagraph] {
        let units = Array(text.utf16)
        var out: [TextParagraph] = []
        var start = 0
        func push(_ end: Int) {
            guard end > start, let raw = String(utf16CodeUnits: Array(units[start..<end]), count: end - start) as String?,
                  !raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            let heading = raw.range(of: #"^#{1,6}\s"#, options: .regularExpression) != nil
            let body = heading ? raw.replacingOccurrences(of: #"^#{1,6}\s+"#, with: "", options: .regularExpression) : raw
            out.append(TextParagraph(start: start, end: end, text: body, heading: heading))
        }
        var i = 0
        let newline: UInt16 = 10
        while i < units.count {
            if units[i] == newline, i + 1 < units.count, units[i + 1] == newline {
                var j = i
                while j < units.count && units[j] == newline { j += 1 }
                push(i)
                start = j
                i = j
            } else { i += 1 }
        }
        push(units.count)
        return out
    }

    /// Sätze eines Absatzes als UTF-16-Bereiche, ohne Rand-Leerzeichen. Stücke, die nur aus
    /// einer Abkürzung bestehen („z. B.“, „Dr.“), bleiben beim folgenden Satz.
    public static func sentences(_ text: String) -> [NSRange] {
        guard !text.isEmpty else { return [] }
        let tokenizer = NLTokenizer(unit: .sentence)
        tokenizer.string = text
        var cuts: [Int] = []  // Beginn jedes Satzes
        var pendingShort = false
        tokenizer.enumerateTokens(in: text.startIndex..<text.endIndex) { range, _ in
            if !pendingShort { cuts.append(NSRange(range, in: text).location) }
            let piece = text[range].trimmingCharacters(in: .whitespacesAndNewlines)
            let last = piece.split(separator: " ").last.map { $0.lowercased() } ?? ""
            pendingShort = abbreviations.contains(last) || (piece.utf16.count < 12 && piece.last == "." && last.utf16.count <= 5)
            return true
        }
        let all = text as NSString
        var out: [NSRange] = []
        for (i, start) in cuts.enumerated() {
            let end = i + 1 < cuts.count ? cuts[i + 1] : all.length
            var a = start, b = end
            while a < b, isSpace(all.character(at: a)) { a += 1 }
            while b > a, isSpace(all.character(at: b - 1)) { b -= 1 }
            if b > a { out.append(NSRange(location: a, length: b - a)) }
        }
        return out
    }

    private static let abbreviations: Set<String> = ["dr.", "prof.", "hr.", "fr.", "st.", "nr.", "ca.", "bzw.", "vgl.", "z.", "b.", "d.", "h.", "u.", "a.", "s.", "mr.", "mrs.", "ms.", "vs.", "e.g.", "i.e."]

    private static func isSpace(_ unit: UInt16) -> Bool {
        unit < 128 ? (unit == 32 || (unit >= 9 && unit <= 13)) : unit == 0xA0
    }

    /// Satz, in dem die Stelle `location` des Absatzes liegt (der letzte davor, falls dazwischen).
    public static func sentenceIndex(_ sentences: [NSRange], at location: Int) -> Int {
        sentences.lastIndex { $0.location <= location } ?? 0
    }

    /// Kurzbuch für Lesen und Vorlesen: Überschriften bleiben, Hervorhebungen fallen weg.
    public static func kurzbuchText(_ markdown: String) -> String {
        markdown.replacingOccurrences(of: "**", with: "").replacingOccurrences(of: "__", with: "")
    }

    /// Sprache aus dem Dateinamen `kurzbuch-<sprache>.md`.
    public static func kurzbuchLanguage(_ file: String?) -> String? {
        guard let file, file.hasPrefix("kurzbuch-"), file.hasSuffix(".md") else { return nil }
        let code = String(file.dropFirst("kurzbuch-".count).dropLast(".md".count))
        return code.range(of: "^[a-z]{2,3}(-[a-z0-9]{2,8})?$", options: .regularExpression) != nil ? code : nil
    }

    /// Vorlesedauer bei normalem Tempo: rund 6 Zeichen je Wort und 150 Wörter je Minute, wie am Desktop.
    public static func listenMilliseconds(chars: Double) -> Double { max(0, chars) / 6 / 150 * 60_000 }

    /// Satz in etwa gleich lange Teile für eine kleine Anzeige, nach Zeichenzahl.
    public static func sentenceParts(_ sentence: String, maxChars: Int) -> [String] {
        guard maxChars > 0 else { return [] }
        return sentenceParts(sentence) { $0.count <= maxChars }
    }

    /// Satz in etwa gleich lange Teile; `fits` sagt, ob ein Text in die Anzeige passt. Geteilt wird an Leerzeichen und
    /// nach Gedankenstrichen, bevorzugt nach einem Satzzeichen. Was allein nicht passt, wird hart geteilt.
    /// Es geht kein Zeichen verloren.
    public static func sentenceParts(_ sentence: String, fits: (String) -> Bool) -> [String] {
        let tokens = partTokens(sentence, fits: fits)
        guard !tokens.isEmpty else { return [] }
        func pack(limit: Int?) -> [String] {
            var parts: [String] = []
            var current = ""
            for token in tokens {
                let next = current + (token.joined || current.isEmpty ? "" : " ") + token.text
                let pause = current.last.map { ",;:—–".contains($0) } ?? false
                let tooLong = limit.map { next.count > $0 } ?? false
                let early = pause && limit.map { current.count * 5 >= $0 * 3 } ?? false
                if !current.isEmpty, !fits(next) || tooLong || early {
                    parts.append(current)
                    current = token.text
                } else {
                    current = next
                }
            }
            if !current.isEmpty { parts.append(current) }
            return parts
        }
        // Erst sehen, wie viele Teile es braucht; dann gleichmäßig füllen statt den letzten kurz zu lassen.
        let greedy = pack(limit: nil)
        guard greedy.count > 1 else { return greedy }
        let total = greedy.reduce(0) { $0 + $1.count }
        let target = (total + greedy.count - 1) / greedy.count
        return pack(limit: target + target / 5)
    }

    private static func partTokens(_ sentence: String, fits: (String) -> Bool) -> [(text: String, joined: Bool)] {
        var tokens: [(text: String, joined: Bool)] = []
        for word in sentence.split(whereSeparator: \.isWhitespace) {
            var first = true
            var rest = Substring(word)
            while !rest.isEmpty {
                // Nach einem Gedankenstrich im Wort darf getrennt werden: „eins—zwei“.
                let end = rest.firstIndex { "—–".contains($0) }.map { rest.index(after: $0) } ?? rest.endIndex
                var piece = rest[..<end]
                rest = rest[end...]
                while !piece.isEmpty {
                    var cut = piece.endIndex
                    while cut > piece.index(after: piece.startIndex), !fits(String(piece[..<cut])) { cut = piece.index(before: cut) }
                    tokens.append((String(piece[..<cut]), !first))
                    first = false
                    piece = piece[cut...]
                }
            }
        }
        return tokens
    }

    /// Welcher Teil gerade dran ist, nach dem Anteil des Satzes, der schon gesprochen sein dürfte.
    public static func sentencePartIndex(_ parts: [String], elapsed: Double, total: Double) -> Int {
        guard parts.count > 1, total > 0 else { return 0 }
        let reached = min(1, max(0, elapsed / total)) * Double(parts.reduce(0) { $0 + $1.count })
        var sum = 0.0
        for (index, part) in parts.enumerated() {
            sum += Double(part.count)
            if reached < sum { return index }
        }
        return parts.count - 1
    }

    /// Favoriten zuerst, sonst bleibt die Reihenfolge, wie `sortViews` am Desktop.
    public static func favoritesFirst(_ books: [LibraryBook]) -> [LibraryBook] {
        books.filter(\.favorite) + books.filter { !$0.favorite }
    }

    /// Sortiert wie die Desktop-Bibliothek (`sortViews`, lva#474): Favoriten davor, untereinander in derselben
    /// Ordnung. `lastUsed` liefert den jüngsten Fortschrittsstempel eines Buchs (0 = nie geöffnet).
    /// Gleichstand fällt auf das Importdatum und dann den Titel zurück, damit die Reihenfolge stabil bleibt
    /// (Altbestand ohne `added_ms` liest sich als 0 und steht bei „Zuletzt hinzugefügt“ am Ende).
    public static func sorted(_ books: [LibraryBook], by sort: LibrarySort,
                              lastUsed: (LibraryBook) -> Double = { _ in 0 }) -> [LibraryBook] {
        let used = sort == .recent ? Dictionary(books.map { ($0.id, lastUsed($0)) }, uniquingKeysWith: max) : [:]
        func byTitle(_ a: LibraryBook, _ b: LibraryBook) -> Bool? {
            let r = sortKey(a.title).compare(sortKey(b.title), locale: Locale(identifier: "de"))
            return r == .orderedSame ? nil : r == .orderedAscending
        }
        func byAdded(_ a: LibraryBook, _ b: LibraryBook) -> Bool? { a.addedMs == b.addedMs ? nil : a.addedMs > b.addedMs }
        func before(_ a: LibraryBook, _ b: LibraryBook) -> Bool {
            if a.favorite != b.favorite { return a.favorite }
            switch sort {
            case .title:
                return byTitle(a, b) ?? (a.id < b.id)
            case .author:
                let r = surname(a.authors).compare(surname(b.authors), locale: Locale(identifier: "de"))
                if r != .orderedSame { return r == .orderedAscending }
                return byTitle(a, b) ?? (a.id < b.id)
            case .added:
                return byAdded(a, b) ?? byTitle(a, b) ?? (a.id < b.id)
            case .recent:
                let ua = used[a.id] ?? 0, ub = used[b.id] ?? 0
                if ua != ub { return ua > ub }
                return byAdded(a, b) ?? byTitle(a, b) ?? (a.id < b.id)
            }
        }
        return books.sorted(by: before)
    }

    /// Vergleichsschlüssel ohne Groß-/Kleinschreibung und Akzente (wie `fold` am Desktop).
    static func sortKey(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de"))
    }

    /// Nachname zum Sortieren: „Theodor Fontane“ → „fontane“, „Kafka, Franz“ → „kafka“.
    static func surname(_ authors: [String]) -> String {
        let a = authors.first ?? ""
        let name = a.contains(",") ? String(a.split(separator: ",").first ?? "") : String(a.split(whereSeparator: \.isWhitespace).last ?? "")
        return sortKey(name.trimmingCharacters(in: .whitespaces))
    }

    /// Index des Absatzes, in dem `chars` liegt (der letzte davor, falls dazwischen).
    public static func paragraphIndex(_ paragraphs: [TextParagraph], at chars: Double) -> Int {
        var lo = 0, hi = paragraphs.count - 1, found = 0
        while lo <= hi {
            let mid = (lo + hi) / 2
            if Double(paragraphs[mid].start) <= chars { found = mid; lo = mid + 1 } else { hi = mid - 1 }
        }
        return found
    }
}

/// Abbildung Zeichenstelle ↔ Hörbuchzeit, wie `makeFollowMap` am Desktop.
public struct FollowMap: Sendable {
    private let points: [LibrarySyncPoint]
    /// Ohne Stützpunkte nur linear geschätzt.
    public let approximate: Bool

    public init(sync: [LibrarySyncPoint], textLength: Double, totalMs: Double) {
        let clean = sync.filter { $0.chars.isFinite && $0.ms.isFinite }.sorted { $0.chars < $1.chars }
        var pts = [LibrarySyncPoint(chars: 0, ms: 0)]
        for p in clean {
            let prev = pts[pts.count - 1]
            if p.chars < prev.chars { continue }
            pts.append(LibrarySyncPoint(chars: p.chars, ms: max(p.ms, prev.ms)))
        }
        let last = pts[pts.count - 1]
        pts.append(LibrarySyncPoint(chars: max(textLength, last.chars), ms: max(totalMs, last.ms)))
        points = pts
        approximate = clean.isEmpty
    }

    public func chars(atMs ms: Double) -> Double { Self.interpolate(points, ms, \.ms, \.chars).rounded() }
    public func ms(atChars chars: Double) -> Double { Self.interpolate(points, chars, \.chars, \.ms) }

    private static func interpolate(_ p: [LibrarySyncPoint], _ value: Double,
                                    _ from: KeyPath<LibrarySyncPoint, Double>, _ to: KeyPath<LibrarySyncPoint, Double>) -> Double {
        if value < p[0][keyPath: from] { return p[0][keyPath: to] }
        let last = p[p.count - 1]
        if value >= last[keyPath: from] { return last[keyPath: to] }
        var lo = 0, hi = p.count - 1
        while hi - lo > 1 { let mid = (lo + hi) / 2; if p[mid][keyPath: from] <= value { lo = mid } else { hi = mid } }
        let a = p[lo], b = p[hi]
        let span = b[keyPath: from] - a[keyPath: from]
        if span <= 0 { return a[keyPath: to] }
        return a[keyPath: to] + (value - a[keyPath: from]) / span * (b[keyPath: to] - a[keyPath: to])
    }
}

/// Stelle im mehrteiligen Hörbuch: welcher Teil, wie weit darin.
public struct TrackPosition: Equatable, Sendable {
    public var index: Int
    public var offsetMs: Double
}

public enum LibraryTracks {
    /// Gesamtzeit → (Teil, Zeit im Teil). Teile ohne Dauer beenden die Rechnung beim ersten unbekannten.
    public static func position(of totalMs: Double, in tracks: [LibraryAudioTrack]) -> TrackPosition {
        var rest = max(0, totalMs)
        for (i, t) in tracks.enumerated() {
            guard let d = t.durationMs else { return TrackPosition(index: i, offsetMs: rest) }
            if rest < d || i == tracks.count - 1 { return TrackPosition(index: i, offsetMs: min(rest, d)) }
            rest -= d
        }
        return TrackPosition(index: 0, offsetMs: 0)
    }

    /// (Teil, Zeit im Teil) → Gesamtzeit.
    public static func total(_ pos: TrackPosition, in tracks: [LibraryAudioTrack]) -> Double {
        tracks.prefix(pos.index).reduce(0) { $0 + ($1.durationMs ?? 0) } + pos.offsetMs
    }
}
