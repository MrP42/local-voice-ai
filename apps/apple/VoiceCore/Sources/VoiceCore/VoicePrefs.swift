import Foundation

/// Vorlesestimme je Buchsprache (lva#352), gleiche Schlüssel und Werte wie am
/// Desktop (`src/lib/reader/voicePrefs.ts`). Die Einträge liegen in
/// `fortschritt.json` und reisen in der Sync-Sammlung `bookmark`:
/// - `tts.voice.lang.<sprache>`: Standardstimme einer Sprache,
/// - `tts.voice.book.<buch>.<sprache>`: Stimme nur für dieses Buch.
///
/// `phonon` (`<sprache>:<name>`, z. B. `de:Anton`) verstehen alle Geräte;
/// `ios`/`desktop` gelten nur auf der eigenen Plattform.
public struct VoicePref: Codable, Equatable, Sendable {
    public var desktop: String?
    public var ios: String?
    public var phonon: String?
    public var updated: Double

    public init(desktop: String? = nil, ios: String? = nil, phonon: String? = nil, updated: Double) {
        self.desktop = desktop; self.ios = ios; self.phonon = phonon; self.updated = updated
    }

    enum CodingKeys: String, CodingKey { case desktop, ios, phonon, updated }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func str(_ k: CodingKeys) -> String? {
            guard let v = try? c.decodeIfPresent(String.self, forKey: k), !v.isEmpty, v.count <= 512 else { return nil }
            return v
        }
        desktop = str(.desktop); ios = str(.ios); phonon = str(.phonon)
        updated = (try? c.decodeIfPresent(Double.self, forKey: .updated)) ?? 0
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(desktop, forKey: .desktop)
        try c.encodeIfPresent(ios, forKey: .ios)
        try c.encodeIfPresent(phonon, forKey: .phonon)
        try c.encode(updated, forKey: .updated)
    }
}

public enum VoicePrefs {
    public static let prefix = "tts.voice."
    public enum Source: Equatable, Sendable { case book, language }

    /// `de`, `en` … aus `de-DE`, `EN` usw.; sonst `nil`.
    public static func language(_ code: String?) -> String? {
        let head = (code ?? "").trimmingCharacters(in: .whitespaces).lowercased()
            .split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init) ?? ""
        guard (2...3).contains(head.count), head.allSatisfy({ $0 >= "a" && $0 <= "z" }) else { return nil }
        return iso639_2[head] ?? head
    }

    /// ISO 639-2 (Open Library: `ger`, `eng` …) auf 639-1, wie `voicePrefs.ts`.
    static let iso639_2: [String: String] = [
        "ger": "de", "deu": "de", "eng": "en", "fre": "fr", "fra": "fr", "spa": "es",
        "ita": "it", "dut": "nl", "nld": "nl", "por": "pt", "pol": "pl", "rus": "ru",
        "swe": "sv", "dan": "da", "nor": "no", "fin": "fi", "cze": "cs", "ces": "cs",
        "tur": "tr", "jpn": "ja", "chi": "zh", "zho": "zh", "kor": "ko", "ukr": "uk",
    ]

    /// Buch, dem ein Bibliothekseintrag gehört: Kurzbücher des iPhones heißen
    /// `<buch>_kurzbuch_<sprache>_<…>`, am PC gilt die Kennung des Buchs.
    public static func owner(_ entryId: String) -> String {
        guard let r = entryId.range(of: "_kurzbuch_") else { return entryId }
        return String(entryId[..<r.lowerBound])
    }

    public static func languageKey(_ lang: String) -> String? { language(lang).map { "\(prefix)lang.\($0)" } }

    public static func bookKey(_ owner: String, _ lang: String) -> String? {
        let o = owner.trimmingCharacters(in: .whitespaces)
        guard !o.isEmpty, o.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_" || $0 == "-") }),
              let l = language(lang) else { return nil }
        return "\(prefix)book.\(o).\(l)"
    }

    public static func decode(_ raw: String?) -> VoicePref? {
        guard let raw, let pref = try? JSONDecoder().decode(VoicePref.self, from: Data(raw.utf8)) else { return nil }
        return pref.desktop != nil || pref.ios != nil || pref.phonon != nil ? pref : nil
    }

    public static func encode(_ pref: VoicePref) -> String? {
        let enc = JSONEncoder(); enc.outputFormatting = [.sortedKeys]
        return (try? enc.encode(pref)).flatMap { String(data: $0, encoding: .utf8) }
    }

    /// iPhone-Wahl als neuer Eintrag. Die Desktop-Kennung bleibt nur, wenn sie
    /// dieselbe Phonon-Stimme meint; sonst läse der PC eine alte Wahl.
    public static func iosPref(id: String, phonon: String?, old: VoicePref?, now: Double) -> VoicePref {
        var pref = VoicePref(ios: id, phonon: phonon, updated: now)
        if let phonon, let old, old.phonon == phonon { pref.desktop = old.desktop }
        return pref
    }

    /// Zuerst die Stimme des Buchs, dann die der Sprache. `toId` übersetzt in
    /// eine hier vorhandene Stimme (`nil`: fehlt – nächste Ebene).
    public static func resolve(owner: String?, language lang: String?, read: (String) -> String?,
                               toId: (VoicePref) -> String?) -> (id: String, source: Source)? {
        guard let lang = language(lang) else { return nil }
        if let owner, let key = bookKey(owner, lang), let pref = decode(read(key)), let id = toId(pref) { return (id, .book) }
        if let key = languageKey(lang), let pref = decode(read(key)), let id = toId(pref) { return (id, .language) }
        return nil
    }

    /// `updated` eines Eintrags, auch einer Aufhebung (`{"updated":…}` ohne Stimme).
    public static func updated(_ raw: String?) -> Double? {
        guard let raw, let pref = try? JSONDecoder().decode(VoicePref.self, from: Data(raw.utf8)) else { return nil }
        return pref.updated
    }

    /// Neuerer Stand gewinnt (Abgleich); gleich alt: der vorhandene bleibt.
    /// Eine Aufhebung zählt wie eine Wahl, sonst bliebe eine Buchausnahme stehen.
    public static func newer(local: String?, remote: String) -> Bool {
        guard let r = updated(remote) else { return false }
        guard let l = updated(local) else { return true }
        return r > l
    }

    /// Hat dieser Eintrag eine Stimme (keine Aufhebung)?
    public static func isSet(_ raw: String?) -> Bool { decode(raw) != nil }
}
