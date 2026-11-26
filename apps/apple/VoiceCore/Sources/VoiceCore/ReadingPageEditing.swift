import Foundation
import CryptoKit

public enum CompositionAction: Sendable { case replace, append, cancel }

/// Die Wahl gilt genau für diesen Text und diese Aufnahmefolge, nicht für spätere Änderungen.
public struct CompositionChoice: Identifiable, Equatable, Sendable {
    public let id = UUID()
    public let pageID: String
    public let currentText: String
    public let proposedText: String
    public let appendedText: String
    public let captures: [ReadingCapture]
}

/// Nur ein Adapter für den Einheits-Player. Dieser Wert wird nie als Bibliotheksbuch gespeichert.
public struct ReadingPagePlaybackSource: Sendable {
    public static let progressPrefix = "reading-page:"
    public let book: LibraryBook
    public let text: String
    public init(page: ReadingPage) {
        var book = LibraryBook(id: "page:" + page.id, title: page.title, addedMs: 0)
        book.text = LibraryTextVersion(pageId: page.id, origin: "page")
        self.book = book; text = page.text
    }
}

/// Gerätelokaler Lesestand, absichtlich außerhalb von `fortschritt.json` und dessen Sync-Schlüsseln.
public enum ReadingPageLocalProgress {
    public static func record(_ key: String, defaults: UserDefaults = .standard) -> ProgressRecord? {
        guard key.hasPrefix(ReadingPagePlaybackSource.progressPrefix), let data = defaults.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(ProgressRecord.self, from: data)
    }
    public static func put(_ key: String, _ record: ProgressRecord, defaults: UserDefaults = .standard) {
        guard key.hasPrefix(ReadingPagePlaybackSource.progressPrefix), let data = try? JSONEncoder().encode(record) else { return }
        defaults.set(data, forKey: key)
    }
}

/// Synchroner Textpuffer. Die Uhr/Planung bleibt beim Aufrufer; alte Entprellungs-
/// Rückrufe dürfen neuere Eingaben nicht vorzeitig speichern.
@MainActor
public final class PendingTextBuffer {
    private var entries: [String: (text: String, revision: Int)] = [:]
    private var revision = 0
    public init() {}
    public var pageIDs: [String] { entries.keys.sorted() }

    @discardableResult
    public func set(_ id: String, text: String) -> Int {
        revision += 1
        entries[id] = (text, revision)
        return revision
    }

    public func flush(_ pageID: String? = nil, save: (String, String) throws -> Void) throws {
        for id in pageID.map({ [$0] }) ?? pageIDs {
            guard let entry = entries[id] else { continue }
            try save(id, entry.text)
            if entries[id]?.revision == entry.revision { entries[id] = nil }
        }
    }

    @discardableResult
    public func flushIfCurrent(_ id: String, revision: Int, save: (String, String) throws -> Void) throws -> Bool {
        guard entries[id]?.revision == revision else { return false }
        try flush(id, save: save)
        return true
    }
}

/// Plattformunabhängige Bearbeitung; OCR liegt beim iPhone-Model.
@MainActor
public final class ReadingPageEditor {
    public let store: ReadingPageStore
    public init(store: ReadingPageStore) { self.store = store }
    public func page(_ id: String) throws -> ReadingPage? { try store.page(id) }

    public func create(text: String, title: String, source: String? = nil) throws -> ReadingPage {
        try Self.validate(text)
        var page = try ReadingPage(title: title)
        page.text = text
        if let source { page.source = Self.intakeSource(source) }
        try store.save(page)
        try store.setCaptures([], for: page.id, composedText: "")
        return page
    }

    public static func intakeSource(_ source: String) -> String {
        switch source {
        case "Fotos (Texterkennung)": return "iPhone-Foto"
        case "Scan (Texterkennung)": return "iPhone-Scan"
        default: return "iPhone-Text"
        }
    }

    /// Stabile Zuordnung auch nach einem Abbruch vor dem Entfernen des Buchs.
    /// Text unverändert übernehmen: Bereinigung würde Lesepositionen verschieben.
    public func convert(book: LibraryBook, text: () -> String?, progress: ProgressRecord? = nil,
                        defaults: UserDefaults = .standard, afterSave: (ReadingPage) throws -> Void) throws -> ReadingPage? {
        guard let version = book.text, version.origin == "document", version.pageId == "ios_" + book.id,
              book.audio == nil, book.editions == nil, book.editionId == nil,
              ["Fotos (Texterkennung)", "Scan (Texterkennung)"].contains(version.sourceName ?? "") else { return nil }
        let digest = SHA256.hash(data: Data(book.id.utf8)).map { String(format: "%02x", $0) }.joined()
        let id = "page_" + String(digest.prefix(59))
        // Eine inzwischen gelöschte Zielseite nicht wiederbeleben.
        guard try store.archivedPage(id) == nil else { return nil }
        let page: ReadingPage
        if let existing = try store.page(id) { page = existing }
        else {
            guard let text = text(), !text.isEmpty else { return nil }
            try Self.validate(text)
            var created = try ReadingPage(id: id, title: book.title)
            created.text = text
            created.source = Self.intakeSource(version.sourceName ?? "")
            // Ein Schreibvorgang; der neue Datensatz enthält bereits keine Aufnahmen.
            try store.save(created)
            page = created
        }
        let key = ReadingPagePlaybackSource.progressPrefix + id
        var progress = progress
        if book.finished {
            if progress == nil {
                let length = Double(page.text.utf16.count)
                progress = ProgressRecord(offset: length, length: length, updated: book.addedMs, unit: .chars)
            }
            progress?.finished = true
        }
        if let progress, ReadingPageLocalProgress.record(key, defaults: defaults) == nil {
            ReadingPageLocalProgress.put(key, progress, defaults: defaults)
            guard ReadingPageLocalProgress.record(key, defaults: defaults) == progress else {
                throw ReadingPageError.persistence
            }
        }
        try afterSave(page)
        return page
    }

    public func updateText(_ id: String, text: String) throws {
        try Self.validate(text)
        guard var page = try page(id) else { throw ReadingPageError.invalid }
        page.text = text
        try store.save(page)
    }

    /// Kein `await` zwischen Prüfung der Berechnungsbasis und atomarem Schreiben.
    public func applyPreparedText(_ id: String, basis: String, text: String) throws {
        guard let page = try page(id), TextAufbereiten.identisch(page.text, basis) else {
            throw TextAufbereiten.Fehler.veraltet
        }
        guard !TextAufbereiten.identisch(text, basis) else { return }
        try updateText(id, text: text)
    }

    public func insert(_ added: [ReadingCapture], into id: String, after captureID: String?) throws -> CompositionChoice? {
        guard try page(id) != nil, !added.isEmpty else { return nil }
        var captures = try store.captures(for: id)
        let index: Int
        if let captureID {
            guard let found = captures.firstIndex(where: { $0.id == captureID }) else { throw ReadingPageError.invalid }
            index = found + 1
        } else { index = captures.count }
        guard captures.count + added.count <= TextInbox.maxImages else { throw ReadingPageError.tooLarge }
        captures.insert(contentsOf: added, at: index)
        return try change(id, captures: captures, appended: ReadingCapture.compose(added))
    }

    public func move(_ id: String, from: IndexSet, to: Int) throws -> CompositionChoice? {
        var captures = try store.captures(for: id)
        guard to >= 0, to <= captures.count, from.allSatisfy({ captures.indices.contains($0) }) else {
            throw ReadingPageError.invalid
        }
        let moving = from.map { captures[$0] }
        for index in from.reversed() { captures.remove(at: index) }
        captures.insert(contentsOf: moving, at: to - from.filter { $0 < to }.count)
        // Umsortieren bringt keinen neuen Text: kein Anhängen anbieten.
        return try change(id, captures: captures, appended: "")
    }

    public func deleteCapture(_ id: String, captureID: String) throws -> CompositionChoice? {
        let captures = try store.captures(for: id).filter { $0.id != captureID }
        // Entfernen bringt keinen neuen Text: kein Anhängen anbieten (sonst doppelte Absätze).
        return try change(id, captures: captures, appended: "")
    }

    public func recognized(_ text: String, pageID: String, captureID: String) throws -> CompositionChoice? {
        guard try page(pageID) != nil else { return nil }
        var captures = try store.captures(for: pageID)
        guard let index = captures.firstIndex(where: { $0.id == captureID }) else { return nil }
        if try store.requiresCompositionChoice(pageID) {
            captures[index].pendingRecognition = text
        } else {
            captures[index].recognizedText = text
        }
        return try change(pageID, captures: captures, appended: text)
    }

    private func change(_ id: String, captures: [ReadingCapture], appended: String) throws -> CompositionChoice? {
        guard var page = try page(id) else { return nil }
        let proposed = ReadingCapture.compose(Self.acceptingRecognition(captures))
        try Self.validate(proposed)
        if try store.requiresCompositionChoice(id) {
            let choice = CompositionChoice(pageID: id, currentText: page.text, proposedText: proposed,
                                           appendedText: appended, captures: captures)
            try store.save(page, captures: captures, composedText: store.composedText(id))
            return choice
        }
        page.text = proposed
        try store.save(page, captures: captures, composedText: proposed)
        return nil
    }

    @discardableResult
    public func resolve(_ choice: CompositionChoice, with action: CompositionAction) throws -> Bool {
        guard var page = try page(choice.pageID), page.text == choice.currentText,
              try store.captures(for: page.id) == choice.captures else { return false }
        var captures = choice.captures
        let baseline: String?
        switch action {
        case .replace:
            page.text = choice.proposedText; baseline = choice.proposedText
            captures = Self.acceptingRecognition(captures)
            for i in captures.indices { captures[i].textEdited = false; captures[i].pendingRecognition = nil }
        case .append:
            page.text = Self.appending(choice.appendedText, to: page.text)
            captures = Self.acceptingRecognition(captures)
            baseline = try store.composedText(page.id)
        case .cancel:
            for i in captures.indices { captures[i].pendingRecognition = nil }
            baseline = try store.composedText(page.id)
        }
        try Self.validate(page.text)
        try store.save(page, captures: captures, composedText: baseline)
        return true
    }

    public static func appending(_ text: String, to original: String) -> String {
        if text.isEmpty { return original }
        return original.isEmpty ? text : original + "\n\n" + text
    }

    private static func acceptingRecognition(_ captures: [ReadingCapture]) -> [ReadingCapture] {
        captures.map { capture in
            var copy = capture
            if let pending = copy.pendingRecognition { copy.recognizedText = pending; copy.pendingRecognition = nil }
            return copy
        }
    }

    public static func validate(_ text: String) throws {
        guard text.utf16.count <= TextIntake.maxCharacters else { throw ReadingPageError.tooLarge }
    }
}
