import Foundation

/// Cover-Suche bei Open Library wie am Desktop (`library.rs`): Es gehen nur Titel
/// und Autor hinaus, nie Buchtext. Ein Treffer zählt nur bei gleichem Kerntitel
/// und, wenn beide Seiten einen Autor kennen, gleichem Nachnamen.
public enum CoverLookup {
    private static let search = "https://openlibrary.org/search.json"
    private static let covers = "https://covers.openlibrary.org/b"

    public static func searchURL(title: String, author: String?) -> URL? {
        var c = URLComponents(string: search)
        var items = [URLQueryItem(name: "title", value: title)]
        if let author, !author.isEmpty { items.append(URLQueryItem(name: "author", value: author)) }
        items.append(URLQueryItem(name: "fields", value: "title,author_name,cover_i"))
        items.append(URLQueryItem(name: "limit", value: "20"))
        c?.queryItems = items
        return c?.url
    }

    static let minTitleWithoutAuthor = 12
    /// Kleiner ist ein Platzhalterbild, größer ein Irrläufer (wie `library.rs`).
    public static func plausibleCover(bytes: Int) -> Bool { bytes >= 512 && bytes <= 8 * 1024 * 1024 }

    public static func coverURL(id: Int) -> URL? { URL(string: "\(covers)/id/\(id)-L.jpg") }

    static func fold(_ s: String) -> String {
        // Umlaute wie am Desktop: „Müller“ und „Mueller“ sind gleich, „Bär“ und „Bar“ nicht.
        var t = s.lowercased()
        for (a, b) in [("ä", "ae"), ("ö", "oe"), ("ü", "ue"), ("ß", "ss")] { t = t.replacingOccurrences(of: a, with: b) }
        return t.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de"))
            .unicodeScalars.filter { CharacterSet.alphanumerics.contains($0) }.map(String.init).joined()
    }

    /// Titel ohne Untertitel und Artikel, nur Buchstaben und Ziffern.
    static func coreTitle(_ title: String) -> String {
        var head = title.split(whereSeparator: { ":|([".contains($0) }).first.map(String.init) ?? title
        for separator in [" – ", " - "] { head = head.components(separatedBy: separator).first ?? head }
        let words = head.split(separator: " ").map(String.init)
        if words.count > 1, ["der", "die", "das", "the", "a", "an", "le", "la", "les"].contains(words[0].lowercased()) {
            return fold(words.dropFirst().joined())
        }
        return fold(head)
    }

    static func surnames(_ authors: [String]) -> Set<String> {
        Set(authors.flatMap { $0.split(whereSeparator: { ",&;".contains($0) }).map(String.init) }
            .flatMap { $0.components(separatedBy: " und ") }
            .compactMap { $0.split(separator: " ").last.map { fold(String($0)) } }
            .filter { $0.count > 1 })
    }

    /// Erstes passendes Cover aus einer `search.json`-Antwort.
    public static func bestCoverId(_ json: Data, title: String, authors: [String]) -> Int? {
        guard let root = try? JSONSerialization.jsonObject(with: json) as? [String: Any],
              let docs = root["docs"] as? [[String: Any]] else { return nil }
        let wanted = coreTitle(title), mine = surnames(authors)
        // Ohne Autor nur bei langem Titel: „Notizen“ darf kein fremdes Cover bekommen.
        guard !wanted.isEmpty, !mine.isEmpty || wanted.count >= minTitleWithoutAuthor else { return nil }
        for doc in docs {
            guard let id = (doc["cover_i"] as? NSNumber)?.intValue, id > 0, let t = doc["title"] as? String,
                  coreTitle(t) == wanted else { continue }
            let theirs = surnames(doc["author_name"] as? [String] ?? [])
            if mine.isEmpty || theirs.isEmpty || !mine.isDisjoint(with: theirs) { return id }
        }
        return nil
    }
}
