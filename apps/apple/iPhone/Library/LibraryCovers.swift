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

    /// Buch nach einem `await` neu lesen: Es kann sich inzwischen geändert haben
    /// (Sync, „fertig“) oder schon ein Cover bekommen haben.
    private func stillMissing(_ id: String) -> LibraryBook? {
        books.first { $0.id == id && coverURL($0) == nil && meta($0).files.cover == nil }
    }

    /// Cover vermerken, ohne den Stand zu erhöhen: Ein neuerer Stand im Konto
    /// gewinnt weiter; gesendet wird trotzdem, weil sich der Inhalt ändert.
    private func noteCover(_ id: String, source: String) {
        guard var book = books.first(where: { $0.id == id }) else { return }
        book.coverSource = source
        try? storeRemote(book, meta: meta(book))
    }

    /// Gibt zurück, ob ein Cover dazugekommen ist.
    @discardableResult func fillMissingCovers() async -> Bool {
        guard !coverSearchRunning else { return false }
        coverSearchRunning = true
        defer { coverSearchRunning = false }
        var changed = false
        for id in books.map(\.id) {
            guard let book = stillMissing(id) else { continue }
            if hasAudio(book) {
                _ = await audiobookInfo(book)  // legt das Bild aus der Datei ab
                if coverURL(book) != nil { noteCover(id, source: "file"); changed = true; continue }
            }
            if searchedRecently(book) { continue }
            do {
                let data = try await Self.findCover(title: book.title, authors: book.authors)
                guard let current = stillMissing(id) else { continue }
                if let data {
                    storeCover(data, for: current)
                    if coverURL(current) != nil { noteCover(id, source: "openlibrary"); changed = true; continue }
                }
                try? Data().write(to: markerURL(current))  // kein Treffer: erst später wieder fragen
            } catch {
                break  // Netz oder Dienst gestört: später erneut, ohne Vermerk
            }
        }
        return changed
    }

    private static func get(_ url: URL) async throws -> Data? {
        var request = URLRequest(url: url, timeoutInterval: 20)
        request.setValue("LocalVoiceAI (Bibliothek; Cover)", forHTTPHeaderField: "User-Agent")
        let (data, response) = try await URLSession.shared.data(for: request)
        let code = (response as? HTTPURLResponse)?.statusCode ?? 0
        if code == 200 { return data }
        if code == 404 { return nil }
        throw URLError(.badServerResponse)  // 429, 5xx: kein „nicht gefunden“
    }

    private static func findCover(title: String, authors: [String]) async throws -> Data? {
        for author in [authors.first, nil] {
            guard let url = CoverLookup.searchURL(title: title, author: author), let json = try await get(url) else { continue }
            if let id = CoverLookup.bestCoverId(json, title: title, authors: authors), let cover = CoverLookup.coverURL(id: id) {
                if let data = try await get(cover), CoverLookup.plausibleCover(bytes: data.count) { return data }
                return nil
            }
            if authors.isEmpty { break }
        }
        return nil
    }
}
