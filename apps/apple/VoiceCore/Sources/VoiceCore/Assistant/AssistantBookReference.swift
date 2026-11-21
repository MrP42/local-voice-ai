import Foundation

/// Buchbezug einer Anfrage; Kennungen stammen ausschließlich aus dem übergebenen Bestand.
public enum AssistantBookReference: Equatable, Sendable {
    case book(String)
    case ask
    case none

    public static func resolve(
        question: String, books: [AssistantItem], lastOpenedBookId: String? = nil,
        lastAnswerBookIds: Set<String>? = nil
    ) -> AssistantBookReference {
        let words = BookTalkTokens.foldedWords(question)
        let text = " " + words.joined(separator: " ") + " "
        let books = books.filter { $0.area == .books }
        let ids = Set(books.map(\.id))
        let articles: Set<String> = [
            "der", "die", "das", "ein", "eine", "the", "a", "an", "buch", "book",
        ]
        func titleWords(_ book: AssistantItem) -> [String] {
            BookTalkTokens.foldedWords(book.title).filter { !articles.contains($0) }
        }
        let fullMatches = books.filter {
            let title = titleWords($0)
            return !title.isEmpty && text.contains(" " + title.joined(separator: " ") + " ")
        }
        // Der längste Volltitel geht enthaltenen Titeln vor („Faust II“ vor „Faust“).
        let longest = fullMatches.map { titleWords($0).count }.max() ?? 0
        let exact = Set(fullMatches.filter { titleWords($0).count == longest }.map(\.id))
        if exact.count > 1 { return .ask }
        if let id = exact.first { return .book(id) }

        let matches = Set(books.filter { book in
            let title = titleWords(book)
            // Einzelne kurze Themenwörter reichen nicht; längere Wörter und Wortfolgen schon.
            if title.contains(where: { $0.count >= 8 && words.contains($0) }) { return true }
            guard title.count >= 2 else { return false }
            for start in 0..<(title.count - 1) {
                let fragment = title[start...(start + 1)].joined(separator: " ")
                if fragment.count >= 10, text.contains(" " + fragment + " ") { return true }
            }
            return false
        }.map(\.id))
        if matches.count > 1 { return .ask }
        if let id = matches.first { return .book(id) }

        let switched = ["anderes thema", "neues thema", "themenwechsel", "anderes buch",
                        "another book", "new topic", "change the subject"]
            .contains { text.contains(" " + $0 + " ") }
        let namedAreas = areas(in: question)
        if !namedAreas.subtracting([.books]).isEmpty { return .none }
        let bookMention = words.contains("buch") || words.contains("book")
        if !switched, let lastAnswerBookIds, !lastAnswerBookIds.isEmpty {
            // Mehrere oder inzwischen verschwundene Quellen sind kein eindeutiger Bezug.
            guard lastAnswerBookIds.count == 1, let id = lastAnswerBookIds.first,
                  ids.contains(id) else { return .ask }
            return .book(id)
        }
        guard bookMention else { return .none }
        if switched { return .ask }
        if let id = lastOpenedBookId, ids.contains(id) { return .book(id) }
        return .ask
    }

    /// Explizite Inhaltsbereiche; keine Änderung am Freigabezustand.
    static func areas(in question: String) -> Set<AssistantArea> {
        let words = Set(BookTalkTokens.foldedWords(question))
        var result = Set<AssistantArea>()
        if !words.isDisjoint(with: ["buch", "book"]) { result.insert(.books) }
        if !words.isDisjoint(with: ["besprechung", "meeting", "transkript"]) {
            result.insert(.transcripts)
        }
        if !words.isDisjoint(with: ["notiz", "note"]) { result.insert(.notes) }
        if !words.isDisjoint(with: ["ablage", "file"]) { result.insert(.files) }
        return result
    }

    public func clarification(english: Bool = false) -> String? {
        self == .ask ? (english ? "Which book do you mean?" : "Welches Buch meinst du?") : nil
    }
}
