import Foundation

// Sprachfassungen eines Buchs (lva#344, Spezifikation docs/superpowers/specs/2026-11-06-sprachfassungen.md).
//
// Typisierte, VERLUSTFREIE Sicht: `LibraryBook` hält `editions`, `primary`, `reading`, `alignments` und
// `edition_tombstones` weiter als rohes `PassthroughJSON`, damit der Abgleich jedes Feld unverändert
// zurückschickt, auch Felder, die nur der PC kennt. `BookEditions` liest daraus eine typisierte Sicht und
// verändert nie etwas. Geschrieben wird nur das Register `reading` (`LibraryBook.settingReading`), und
// zwar als frisches Objekt.

// MARK: - JSON-Zugriff

extension PassthroughJSON {
    public subscript(_ key: String) -> PassthroughJSON? {
        if case .object(let o) = self { return o[key] }
        return nil
    }
    public var stringValue: String? { if case .string(let s) = self { return s }; return nil }
    public var numberValue: Double? { if case .number(let n) = self { return n }; return nil }
    public var boolValue: Bool? { if case .bool(let b) = self { return b }; return nil }
    public var arrayValue: [PassthroughJSON]? { if case .array(let a) = self { return a }; return nil }
    public var objectValue: [String: PassthroughJSON]? { if case .object(let o) = self { return o }; return nil }

    /// Kopie mit gesetztem (oder entferntem) Schlüssel; ein Nicht-Objekt wird zum Objekt.
    public func setting(_ key: String, _ value: PassthroughJSON?) -> PassthroughJSON {
        var o = objectValue ?? [:]
        o[key] = value
        return .object(o)
    }

    public init<T: Encodable>(encoding value: T) throws {
        let data = try JSONEncoder().encode(value)
        self = try JSONDecoder().decode(PassthroughJSON.self, from: data)
    }

    public func decoded<T: Decodable>(_ type: T.Type) -> T? {
        guard let data = try? JSONEncoder().encode(self) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }
}

// MARK: - Modell

/// Eine Sprachfassung: Text und/oder Hörbuch in einer Sprache.
public struct BookEdition: Equatable, Sendable, Identifiable {
    public var id: String
    /// Sprachcode (`de`, `en`), leer = unbekannt.
    public var lang: String
    public var title: String
    public var subtitle: String?
    public var text: LibraryTextVersion?
    public var audio: LibraryAudioVersion?
    /// Dauerhafte Fortschrittsreferenz des Hörbuchs; wird nie umgeschlüsselt.
    public var audioKey: String
    public var origin: String
    public var originBook: String?
    public var addedMs: Double
    public var updated: Double
    public var device: String
    /// Das Objekt, wie es im Buch steht.
    public var json: PassthroughJSON

    public init(id: String, lang: String, title: String, subtitle: String? = nil, text: LibraryTextVersion? = nil,
                audio: LibraryAudioVersion? = nil, audioKey: String, origin: String = "", originBook: String? = nil,
                addedMs: Double = 0, updated: Double = 0, device: String = "", json: PassthroughJSON = .object([:])) {
        self.id = id; self.lang = lang; self.title = title; self.subtitle = subtitle; self.text = text; self.audio = audio
        self.audioKey = audioKey; self.origin = origin; self.originBook = originBook; self.addedMs = addedMs
        self.updated = updated; self.device = device; self.json = json
    }

    /// Aus dem rohen Objekt; fehlt `audio_key`, gilt die Regel des Kerns (`audio:<buch>:<fassung>`).
    public init?(json: PassthroughJSON, bookId: String) {
        guard let id = json["id"]?.stringValue, !id.isEmpty else { return nil }
        let key = json["audio_key"]?.stringValue ?? ""
        self.init(id: id,
                  lang: json["lang"]?.stringValue ?? "",
                  title: json["title"]?.stringValue ?? "",
                  subtitle: json["subtitle"]?.stringValue,
                  text: json["text"]?.decoded(LibraryTextVersion.self),
                  audio: json["audio"]?.decoded(LibraryAudioVersion.self),
                  audioKey: key.isEmpty ? "audio:\(bookId):\(id)" : key,
                  origin: json["origin"]?.stringValue ?? "",
                  originBook: json["origin_book"]?.stringValue,
                  addedMs: json["added_ms"]?.numberValue ?? 0,
                  updated: json["updated"]?.numberValue ?? 0,
                  device: json["device"]?.stringValue ?? "",
                  json: json)
    }

    /// Fortschrittsschlüssel des Textes (Reiter „original“); `nil` ohne Text.
    public var textKey: String? { text.map { LibraryProgress.textKey(pageId: $0.pageId) } }

    /// Gehört dieser Fortschrittsschlüssel zu dieser Fassung?
    public func owns(key: String) -> Bool {
        if key == audioKey { return true }
        guard let text else { return false }
        return key.hasPrefix("text:\(text.pageId):")
    }

    /// Eigennamen der Fassung wie am PC: „Deutsch“, „English“.
    public var languageName: String { BookEditions.languageName(lang) }
    public var languageShort: String { BookEditions.languageShort(lang) }
}

/// Stempel eines Registers (`primary`).
public struct EditionStamp: Equatable, Sendable {
    public var edition: String
    public var updated: Double
    public var device: String
    public init(edition: String, updated: Double, device: String = "") {
        self.edition = edition; self.updated = updated; self.device = device
    }
    public init?(json: PassthroughJSON) {
        guard let e = json["edition"]?.stringValue else { return nil }
        self.init(edition: e, updated: json["updated"]?.numberValue ?? 0, device: json["device"]?.stringValue ?? "")
    }
}

/// Stelle, mit der gewechselt wurde: Schlüssel der Zielfassung, Einheit (`ms` oder `chars`), Stelle.
public struct ReadingPositionRef: Equatable, Sendable {
    public var key: String
    public var unit: String
    public var offset: Double
    public init(key: String, unit: String, offset: Double) { self.key = key; self.unit = unit; self.offset = offset }
    public init?(json: PassthroughJSON) {
        guard let key = json["key"]?.stringValue, let offset = json["offset"]?.numberValue else { return nil }
        self.init(key: key, unit: json["unit"]?.stringValue ?? "chars", offset: offset)
    }
    public var json: PassthroughJSON {
        .object(["key": .string(key), "unit": .string(unit), "offset": .number(offset)])
    }
}

/// Zuletzt gewählte Sprachfassung, geräteübergreifend, samt Stelle (Register `reading`).
public struct ReadingState: Equatable, Sendable {
    public var edition: String
    public var updated: Double
    public var device: String
    public var position: ReadingPositionRef?
    public init(edition: String, updated: Double, device: String = "", position: ReadingPositionRef? = nil) {
        self.edition = edition; self.updated = updated; self.device = device; self.position = position
    }
    public init?(json: PassthroughJSON) {
        guard let e = json["edition"]?.stringValue else { return nil }
        self.init(edition: e, updated: json["updated"]?.numberValue ?? 0, device: json["device"]?.stringValue ?? "",
                  position: json["position"].flatMap(ReadingPositionRef.init(json:)))
    }
    /// Wie das Register im Buch steht; ohne Stelle fehlt `position` (wie im Kern).
    public var json: PassthroughJSON {
        var o: [String: PassthroughJSON] = ["edition": .string(edition), "updated": .number(updated), "device": .string(device)]
        if let position { o["position"] = position.json }
        return .object(o)
    }
    /// Ordnung der Register: (updated, device, edition), das größere Tripel gewinnt (wie `Stamp::beats`).
    public func isNewer(than other: ReadingState) -> Bool {
        if updated != other.updated { return updated > other.updated }
        if device != other.device { return device > other.device }
        return edition > other.edition
    }
}

/// Zusammenhängende Kapitel (0-basiert) in `a` und `b`.
public struct AlignGroup: Equatable, Sendable {
    public var a: [Int]
    public var b: [Int]
    public var confirmed: Bool
    public init(a: [Int], b: [Int], confirmed: Bool) { self.a = a; self.b = b; self.confirmed = confirmed }
    init?(json: PassthroughJSON) {
        guard let a = json["a"]?.arrayValue, let b = json["b"]?.arrayValue else { return nil }
        let ia = a.compactMap { $0.numberValue.map { Int($0) } }, ib = b.compactMap { $0.numberValue.map { Int($0) } }
        guard ia.count == a.count, ib.count == b.count else { return nil }
        self.init(a: ia, b: ib, confirmed: json["confirmed"]?.boolValue ?? false)
    }
}

/// Kapitelzuordnung zweier Fassungen, lückenlos in Gruppen.
public struct EditionAlignment: Equatable, Sendable {
    public var a: String
    public var b: String
    public var updated: Double
    public var device: String
    public var groups: [AlignGroup]
    public init(a: String, b: String, updated: Double, device: String = "", groups: [AlignGroup]) {
        self.a = a; self.b = b; self.updated = updated; self.device = device; self.groups = groups
    }
    init?(json: PassthroughJSON) {
        guard let a = json["a"]?.stringValue, let b = json["b"]?.stringValue, let raw = json["groups"]?.arrayValue else { return nil }
        let groups = raw.compactMap(AlignGroup.init(json:))
        guard groups.count == raw.count else { return nil }
        self.init(a: a, b: b, updated: json["updated"]?.numberValue ?? 0, device: json["device"]?.stringValue ?? "", groups: groups)
    }
}

/// Typisierte, nur lesende Sicht auf den Fassungsteil eines Buchs.
public struct BookEditions: Equatable, Sendable {
    public let bookId: String
    /// Lebende Fassungen (Grabsteine abgezogen), in der Reihenfolge des Buchs.
    public let editions: [BookEdition]
    /// Kennung → Zeitpunkt der Löschung.
    public let tombstones: [String: Double]
    /// Roh gespeichertes Register der Hauptfassung.
    public let primaryStamp: EditionStamp?
    /// Wirksame Auswahl: ihre Fassung lebt, und die Stelle gehört zu ihr.
    public let reading: ReadingState?
    public let alignments: [EditionAlignment]
    /// Schema 1 ohne `editions`: die eine Fassung ist aus `text`/`audio`/`language` abgeleitet (`ed_<buch>`).
    public let synthesized: Bool

    public init(book: LibraryBook) {
        bookId = book.id
        var dead: [String: Double] = [:]
        for t in book.editionTombstones?.arrayValue ?? [] {
            guard let id = t["id"]?.stringValue else { continue }
            dead[id] = max(dead[id] ?? 0, t["removed_ms"]?.numberValue ?? 0)
        }
        tombstones = dead
        let raw = book.editions?.arrayValue ?? []
        var seen = Set<String>()
        var list = raw.compactMap { BookEdition(json: $0, bookId: book.id) }
            .filter { dead[$0.id] == nil && seen.insert($0.id).inserted }
        // Wie `normalize` im Kern: nur Schema 1 wird abgeleitet, ein Buch mit Grabsteinen war schon Schema 2.
        let derive = raw.isEmpty && dead.isEmpty && (book.text != nil || book.audio != nil)
        if derive {
            list = [BookEdition(id: Self.migratedId(book.id), lang: Self.languageCode(book.language), title: book.title,
                                subtitle: book.subtitle, text: book.text, audio: book.audio,
                                audioKey: LibraryProgress.audioKey(bookId: book.id), origin: "migration",
                                addedMs: book.addedMs, updated: book.addedMs)]
        }
        editions = list
        synthesized = derive
        primaryStamp = book.primary.flatMap(EditionStamp.init(json:))
        alignments = (book.alignments?.arrayValue ?? []).compactMap(EditionAlignment.init(json:))
        let chosen = book.reading.flatMap(ReadingState.init(json:))
        if var r = chosen, let e = list.first(where: { $0.id == r.edition }) {
            if let p = r.position, !e.owns(key: p.key) { r.position = nil }
            reading = r
        } else {
            reading = nil
        }
    }

    /// Kennung der einzigen Fassung eines Schema-1-Buchs (wie `migrated_id` im Kern).
    public static func migratedId(_ bookId: String) -> String { "ed_\(bookId)" }

    public func edition(_ id: String?) -> BookEdition? {
        guard let id else { return nil }
        return editions.first { $0.id == id }
    }

    /// Mehr als eine Sprachfassung?
    public var hasLanguages: Bool { editions.count > 1 }

    /// Wirksame Hauptfassung: das Register, falls es auf eine lebende Fassung zeigt, sonst die älteste
    /// nach (`added_ms`, Kennung). Das Register selbst wird nie repariert.
    public var primaryId: String? {
        if let p = primaryStamp?.edition, editions.contains(where: { $0.id == p }) { return p }
        return editions.min { ($0.addedMs, $0.id) < ($1.addedMs, $1.id) }?.id
    }
    public var primary: BookEdition? { edition(primaryId) }

    /// Zuletzt aktive Fassung (`reading`), sonst die Hauptfassung.
    public var readingEdition: BookEdition? { edition(reading?.edition) ?? primary }

    /// Zuordnung des Paars in Richtung `from` → `to`: die jüngste gespeicherte, Gruppen gedreht, falls sie
    /// andersherum steht. Nur zwischen lebenden Fassungen.
    public func alignment(from: String, to: String) -> (alignment: EditionAlignment?, groups: [AlignGroup]?) {
        let found = alignments
            .filter { ($0.a == from && $0.b == to) || ($0.a == to && $0.b == from) }
            .filter { edition($0.a) != nil && edition($0.b) != nil }
            .max { $0.updated < $1.updated }
        guard let found else { return (nil, nil) }
        if found.a == from { return (found, found.groups) }
        return (found, found.groups.map { AlignGroup(a: $0.b, b: $0.a, confirmed: $0.confirmed) })
    }

    // MARK: Sprache

    private static let aliases: [String: String] = [
        "deu": "de", "ger": "de", "deutsch": "de", "german": "de",
        "eng": "en", "englisch": "en", "english": "en",
        "fra": "fr", "fre": "fr", "french": "fr", "français": "fr", "francais": "fr", "französisch": "fr",
        "spa": "es", "spanish": "es", "español": "es", "spanisch": "es",
        "ita": "it", "italian": "it", "italiano": "it", "italienisch": "it",
    ]

    /// Sprachcode aus freier Angabe („Deutsch“, „de-DE“, „ger“) oder leer.
    public static func languageCode(_ raw: String?) -> String {
        let head = (raw ?? "").trimmingCharacters(in: .whitespaces).lowercased()
            .split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init) ?? ""
        if let alias = aliases[head] { return alias }
        if head.utf8.count == 2, head.utf8.allSatisfy({ $0 >= 0x61 && $0 <= 0x7a }) { return head }
        return ""
    }

    /// Kürzel zur Anzeige („DE“); unbekannte Sprache: Gedankenstrich.
    public static func languageShort(_ code: String?) -> String {
        let c = languageCode(code)
        return c.isEmpty ? "–" : c.uppercased()
    }

    /// Eigenname der Sprache („Deutsch“, „English“, „Français“); unbekannt: das Kürzel.
    public static func languageName(_ code: String?) -> String {
        let c = languageCode(code)
        guard !c.isEmpty else { return "" }
        guard let name = Locale(identifier: c).localizedString(forLanguageCode: c), !name.isEmpty else { return c.uppercased() }
        return name.prefix(1).uppercased() + name.dropFirst()
    }
}

// MARK: - Schreiben (nur das Register `reading`)

extension LibraryBook {
    /// Auswahl setzen wie `set_reading` im Kern: Der Zeitstempel ist `max(jetzt, vorher + 1)`, damit die
    /// Schreibung auch in derselben Millisekunde echt jünger ist als der bisherige Stand. Eine Stelle, die
    /// nicht zur Fassung gehört, entfällt (wie `effective_reading`). `nil`, wenn die Fassung nicht lebt
    /// oder das Buch keine Fassungen führt (Schema 1). Alle anderen Felder bleiben unberührt.
    public func settingReading(edition id: String, position: ReadingPositionRef?, device: String, now: Double) -> LibraryBook? {
        let view = BookEditions(book: self)
        guard !view.synthesized, let target = view.edition(id) else { return nil }
        var kept = position
        if let p = kept, !target.owns(key: p.key) { kept = nil }
        let before = reading?["updated"]?.numberValue
        var out = self
        out.reading = ReadingState(edition: id, updated: before.map { max(now, $0 + 1) } ?? now, device: device, position: kept).json
        if out.schema == nil { out.schema = 2 }
        return out
    }

    /// Nach einem Empfang (`self` = Stand aus dem Konto): Hat dieses Gerät die Auswahl später gesetzt und
    /// noch nicht gesendet, bleibt sie. Sonst ginge der Wechsel verloren, bevor der PC ihn je gesehen hat.
    public func keepingNewerReading(from local: LibraryBook) -> LibraryBook {
        guard let mine = local.reading.flatMap(ReadingState.init(json:)),
              BookEditions(book: self).edition(mine.edition) != nil else { return self }
        if let theirs = reading.flatMap(ReadingState.init(json:)), !mine.isNewer(than: theirs) { return self }
        var out = self
        out.reading = local.reading
        return out
    }
}

extension BookEditions {
    /// Zu welcher Fassung gehören die flachen Dateien (`files.text/cover/audio`) des eingehenden Buchs? Wie
    /// `editions::sender_primary` im Kern: Ein Altgerät (Schema < 2, ohne Fassungen) liefert die Arbeitskopie der
    /// hiesigen Hauptfassung, sonst gilt die Hauptfassung des (normalisierten) eingehenden Buchs; Altbestand aus
    /// Schema 1 gehört zu `ed_<buch>`.
    public static func senderPrimary(local: LibraryBook?, incoming: LibraryBook) -> String? {
        let legacy = (incoming.schema ?? 1) < 2 && incoming.editions == nil
        if legacy, let local {
            let mine = BookEditions(book: local)
            if !mine.synthesized { return mine.primaryId }
        }
        return BookEditions(book: incoming).primaryId
    }
}

extension LibraryBook {
    /// Kommt das Buch von einem Altgerät (Schema < 2, ohne Fassungen), behält es die hiesigen Fassungen samt
    /// Hauptfassung, Lesestand und Zuordnungen, damit der Abgleich sie nicht löscht.
    public func keepingEditions(from local: LibraryBook) -> LibraryBook {
        guard (schema ?? 1) < 2, editions == nil, local.editions != nil else { return self }
        var out = self
        out.schema = local.schema; out.workKey = local.workKey; out.editions = local.editions; out.primary = local.primary
        out.editionTombstones = local.editionTombstones; out.reading = local.reading; out.alignments = local.alignments
        return out
    }
}

#if canImport(CryptoKit)
// MARK: - Dateien je Fassung (`files.editions[<fassung>]`)

/// Dateiverweise einer Fassung im Konto.
public struct EditionFiles: Equatable, Sendable {
    public var text: BlobRef?
    public var cover: BlobRef?
    public var audio: [BlobRef]
    public init(text: BlobRef? = nil, cover: BlobRef? = nil, audio: [BlobRef] = []) {
        self.text = text; self.cover = cover; self.audio = audio
    }
}

extension BookFiles {
    /// Dateien der Fassung `id` aus `files.editions`; leer, wenn es keinen Eintrag gibt.
    /// Die flachen Felder `text`/`cover`/`audio` gehören zur Hauptfassung des Senders und stehen hier nicht.
    public func files(forEdition id: String) -> EditionFiles {
        guard let entry = editions?[id] else { return EditionFiles() }
        return EditionFiles(text: entry["text"]?.decoded(BlobRef.self),
                            cover: entry["cover"]?.decoded(BlobRef.self),
                            audio: (entry["audio"]?.arrayValue ?? []).compactMap { $0.decoded(BlobRef.self) })
    }

    /// Dateiverweise nach dem Empfang von einem Altgerät (Schema < 2, ohne Fassungen); `self` ist der bisherige Stand.
    /// Die flachen Felder des Senders gelten für seine Hauptfassung `senderFlat`. Alles Übrige bleibt: Verweise
    /// weiterer Fassungen in `files.editions`, und flache Dateien einer anderen bisherigen Flat-Fassung `localFlat`
    /// wandern feldweise dorthin (ein umziehendes Feld gewinnt, fehlt es, bleibt der vorhandene Wert; unbekannte
    /// Felder bleiben). Fehlt dem Sender ein flaches Feld, bleibt der bisherige Verweis der Fassung stehen, auch der
    /// bisherige flache, wenn `localFlat == senderFlat`. `live`: nur lebende Fassungen (keine Grabsteine) behalten
    /// Verweise; `nil` filtert nicht.
    public func receivingLegacy(_ incoming: BookFiles, localFlat: String?, senderFlat: String?, live: Set<String>? = nil) -> BookFiles {
        var out = incoming
        var entries = editions?.objectValue ?? [:]
        if let lf = localFlat, lf != senderFlat, text != nil || cover != nil || !audio.isEmpty {
            var entry = entries[lf]?.objectValue ?? [:]
            if let t = text, let j = try? PassthroughJSON(encoding: t) { entry["text"] = j }
            if let c = cover, let j = try? PassthroughJSON(encoding: c) { entry["cover"] = j }
            if !audio.isEmpty, let j = try? PassthroughJSON(encoding: audio) { entry["audio"] = j }
            entries[lf] = .object(entry)
        }
        if let sf = senderFlat {
            var known = EditionFiles(text: entries[sf]?["text"]?.decoded(BlobRef.self), cover: entries[sf]?["cover"]?.decoded(BlobRef.self),
                                     audio: (entries[sf]?["audio"]?.arrayValue ?? []).compactMap { $0.decoded(BlobRef.self) })
            if localFlat == sf {
                known.text = known.text ?? text
                known.cover = known.cover ?? cover
                if known.audio.isEmpty { known.audio = audio }
            }
            if out.text == nil { out.text = known.text }
            if out.cover == nil { out.cover = known.cover }
            if out.audio.isEmpty { out.audio = known.audio }
            entries[sf] = nil
        }
        if let live { entries = entries.filter { live.contains($0.key) } }
        out.editions = entries.isEmpty ? nil : .object(entries)
        return out
    }

    /// Dateien der Fassung `id`, wenn die flachen Felder zur Fassung `flat` gehören (Hauptfassung des Senders).
    public func files(forEdition id: String, flat: String?) -> EditionFiles {
        guard let flat, flat == id else { return files(forEdition: id) }
        return EditionFiles(text: text, cover: cover, audio: audio)
    }
}
#endif
