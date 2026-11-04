import Foundation

/// Fehlende Titelbilder ergänzen, damit jedes Buch eins hat wie am Desktop:
/// erst aus der Hörbuchdatei, sonst über Open Library. Es gehen nur Titel und
/// Autor hinaus. Ein Cover aus dem Konto hat Vorrang und kommt über den Sync.
extension PhoneLibrary {
    private static let retryAfter: TimeInterval = 14 * 86_400

    private func markerURL(_ book: LibraryBook) -> URL { dir(book).appendingPathComponent("cover-gesucht") }

    private func searchedRecently(_ book: LibraryBook) -> Bool {
        guard let date = (try? FileManager.default.attributesOfItem(atPath: markerURL(book).path))?[.modificationDate] as? Date else { return false }
        return Date().timeIntervalSince(date) < Self.retryAfter
    }

    func fillMissingCovers() async {
        guard !coverSearchRunning else { return }
        coverSearchRunning = true
        defer { coverSearchRunning = false }
        for book in books where coverURL(book) == nil && meta(book).files.cover == nil {
            if hasAudio(book) {
                _ = await audiobookInfo(book)  // legt das Bild aus der Datei ab
                if coverURL(book) != nil { continue }
            }
            if searchedRecently(book) { continue }
            do {
                if let data = try await Self.findCover(title: book.title, authors: book.authors) {
                    storeCover(data, for: book)
                    if coverURL(book) != nil {
                        var copy = book; copy.coverSource = "openlibrary"
                        var m = meta(book); m.updated = Self.nowMs()
                        try? storeRemote(copy, meta: m)
                        continue
                    }
                }
                try? Data().write(to: markerURL(book))  // kein Treffer: erst später wieder fragen
            } catch {
                return  // kein Netz: nächster Start versucht es erneut, ohne Vermerk
            }
        }
    }

    private static func get(_ url: URL) async throws -> Data? {
        var request = URLRequest(url: url, timeoutInterval: 20)
        request.setValue("LocalVoiceAI (Bibliothek; Cover)", forHTTPHeaderField: "User-Agent")
        let (data, response) = try await URLSession.shared.data(for: request)
        return (response as? HTTPURLResponse)?.statusCode == 200 ? data : nil
    }

    private static func findCover(title: String, authors: [String]) async throws -> Data? {
        for author in [authors.first, nil] {
            guard let url = CoverLookup.searchURL(title: title, author: author), let json = try await get(url) else { continue }
            if let id = CoverLookup.bestCoverId(json, title: title, authors: authors), let cover = CoverLookup.coverURL(id: id) {
                return try await get(cover)
            }
            if authors.isEmpty { break }
        }
        return nil
    }
}
