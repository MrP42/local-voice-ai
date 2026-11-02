import Foundation
import AVFoundation
import PDFKit
import UniformTypeIdentifiers

/// Bibliothek auf dem iPhone (Epic #210). Ablage wie am Desktop:
/// `Documents/Library/<id>/buch.json`, `<id>/text.txt`, `<id>/audio/…`,
/// `fortschritt.json` (Schlüssel → JSON-Text des Fortschrittsdatensatzes).
@MainActor
final class PhoneLibrary: ObservableObject {
    static let shared = PhoneLibrary()

    @Published private(set) var books: [LibraryBook] = []
    @Published private(set) var progress: [String: String] = [:]
    @Published private(set) var importing = false
    @Published var error: String?

    let root: URL
    private var saveTask: Task<Void, Never>?

    init(root: URL? = nil) {
        self.root = root ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Library")
        try? FileManager.default.createDirectory(at: self.root, withIntermediateDirectories: true,
                                                 attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        reload()
    }

    // MARK: Lesen

    func reload() {
        let fm = FileManager.default
        let dirs = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        var list: [LibraryBook] = []
        for dir in dirs where dir.hasDirectoryPath || (try? dir.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
            guard let data = try? Data(contentsOf: dir.appendingPathComponent("buch.json")),
                  let book = try? JSONDecoder().decode(LibraryBook.self, from: data) else { continue }
            list.append(book)
        }
        books = list.sorted { $0.addedMs > $1.addedMs }
        if let data = try? Data(contentsOf: progressURL), let map = try? JSONDecoder().decode([String: String].self, from: data) {
            progress = map
        } else if FileManager.default.fileExists(atPath: progressURL.path) {
            // Unlesbar: beiseitelegen statt beim nächsten Sichern überschreiben (wie am Desktop).
            let aside = root.appendingPathComponent("fortschritt.json.defekt-\(Int(Date().timeIntervalSince1970 * 1000))")
            try? FileManager.default.moveItem(at: progressURL, to: aside)
            progress = [:]
        }
    }

    var progressURL: URL { root.appendingPathComponent("fortschritt.json") }
    func dir(_ book: LibraryBook) -> URL { root.appendingPathComponent(book.id, isDirectory: true) }
    func textURL(_ book: LibraryBook) -> URL { dir(book).appendingPathComponent("text.txt") }
    func hasText(_ book: LibraryBook) -> Bool { FileManager.default.fileExists(atPath: textURL(book).path) }
    func text(_ book: LibraryBook) -> String? { try? String(contentsOf: textURL(book), encoding: .utf8) }

    /// Hörbuchteile in Hörreihenfolge, nur die vorhandenen.
    func audioURLs(_ book: LibraryBook) -> [URL] {
        guard let audio = book.audio else { return [] }
        return audio.allTracks.map { dir(book).appendingPathComponent("audio").appendingPathComponent($0.file) }
            .filter { FileManager.default.fileExists(atPath: $0.path) }
    }
    func hasAudio(_ book: LibraryBook) -> Bool { !(book.audio?.allTracks.isEmpty ?? true) && !audioURLs(book).isEmpty }

    func record(_ key: String) -> ProgressRecord? {
        guard let raw = progress[key] else { return nil }
        return try? JSONDecoder().decode(ProgressRecord.self, from: Data(raw.utf8))
    }

    func textKey(_ book: LibraryBook) -> String { LibraryProgress.textKey(pageId: book.text?.pageId ?? book.id) }
    func audioKey(_ book: LibraryBook) -> String { LibraryProgress.audioKey(bookId: book.id) }

    // MARK: Schreiben

    func put(_ key: String, _ rec: ProgressRecord) {
        let merged = record(key).map { LibraryProgress.merge(rec, $0) } ?? rec
        guard let data = try? JSONEncoder().encode(merged), let text = String(data: data, encoding: .utf8) else { return }
        progress[key] = text
        scheduleSave()
    }

    func setFinished(_ book: LibraryBook, _ finished: Bool) {
        var copy = book; copy.finished = finished
        try? write(copy)
        reload()
    }

    func flush() {
        saveTask?.cancel(); saveTask = nil
        writeProgress()
    }

    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 800_000_000)
            guard !Task.isCancelled else { return }
            self?.writeProgress()
        }
    }

    private func writeProgress() {
        guard let data = try? JSONEncoder().encode(progress) else { return }
        try? data.write(to: progressURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    private func write(_ book: LibraryBook) throws {
        let enc = JSONEncoder(); enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(at: dir(book), withIntermediateDirectories: true)
        try enc.encode(book).write(to: dir(book).appendingPathComponent("buch.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    func delete(_ book: LibraryBook) {
        try? FileManager.default.removeItem(at: dir(book))
        for key in [textKey(book), audioKey(book)] { progress[key] = nil }
        writeProgress()
        reload()
    }

    // MARK: Import

    static let textTypes: [UTType] = [.epub, .pdf, .plainText, UTType(filenameExtension: "md") ?? .plainText]
    static let audioTypes: [UTType] = [.mp3, .mpeg4Audio, .audio, UTType(filenameExtension: "m4b") ?? .audio]
    static var importTypes: [UTType] { textTypes + audioTypes }

    enum Kind { case text, audio, drm, unknown }
    static func kind(of url: URL) -> Kind {
        switch url.pathExtension.lowercased() {
        case "epub", "pdf", "txt", "md", "markdown": return .text
        case "mp3", "m4a", "m4b", "aac", "wav", "flac", "ogg", "opus": return .audio
        case "aax", "aa", "aaxc": return .drm
        default: return .unknown
        }
    }

    /// Dateien aus dem Dateiwähler. Mehrere Audiodateien zusammen werden ein
    /// mehrteiliges Hörbuch (nach Dateinamen sortiert); jede Textdatei ein Buch.
    /// Passt ein neues Buch zu einem vorhandenen (gleicher Titel), wird die
    /// Fassung angehängt statt einen zweiten Eintrag anzulegen.
    func importFiles(_ urls: [URL]) async {
        importing = true; defer { importing = false; reload() }
        error = nil
        var audio: [URL] = []
        var problems: [String] = []
        for url in urls {
            switch Self.kind(of: url) {
            case .text:
                do { try await importText(url) } catch { problems.append(url.lastPathComponent) }
            case .audio: audio.append(url)
            case .drm: problems.append("\(url.lastPathComponent) (DRM-geschützt, z. B. Audible AAX)")
            case .unknown: problems.append("\(url.lastPathComponent) (Format unbekannt)")
            }
        }
        if !audio.isEmpty {
            do { try await importAudio(audio.sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedAscending }) }
            catch { problems.append("Hörbuch: \(error.localizedDescription)") }
        }
        if !problems.isEmpty { error = "Nicht importiert: " + problems.joined(separator: ", ") }
    }

    private func access<T>(_ url: URL, _ body: () throws -> T) rethrows -> T {
        let ok = url.startAccessingSecurityScopedResource()
        defer { if ok { url.stopAccessingSecurityScopedResource() } }
        return try body()
    }

    private func importText(_ url: URL) async throws {
        let ext = url.pathExtension.lowercased()
        let extracted: ExtractedBook = try access(url) {
            let data = try Data(contentsOf: url)
            switch ext {
            case "epub": return try BookText.epub(data)
            case "pdf":
                guard let pdf = PDFDocument(data: data) else { throw BookTextError.corrupt("PDF") }
                let pages = (0..<pdf.pageCount).compactMap { pdf.page(at: $0)?.string?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
                guard !pages.isEmpty else { throw BookTextError.empty }
                let attrs = pdf.documentAttributes
                return ExtractedBook(title: attrs?[PDFDocumentAttribute.titleAttribute] as? String,
                                     authors: (attrs?[PDFDocumentAttribute.authorAttribute] as? String).map { [$0] } ?? [], language: nil,
                                     text: pages.joined(separator: "\n\n"))
            default: return ExtractedBook(title: nil, authors: [], language: nil, text: try BookText.plain(data))
            }
        }
        let title = (extracted.title?.isEmpty == false ? extracted.title! : url.deletingPathExtension().lastPathComponent)
        var book = match(title: title) ?? LibraryBook(id: Self.newId(), title: title, authors: extracted.authors,
                                                       coverSource: "generated", addedMs: Self.nowMs())
        if book.authors.isEmpty { book.authors = extracted.authors }
        if book.language == nil { book.language = extracted.language }
        book.text = LibraryTextVersion(pageId: "ios_\(book.id)", origin: "document", sourceName: url.lastPathComponent)
        try write(book)
        try extracted.text.write(to: textURL(book), atomically: true, encoding: .utf8)
    }

    private func importAudio(_ urls: [URL]) async throws {
        let title = Self.commonTitle(urls)
        var book = match(title: title) ?? LibraryBook(id: Self.newId(), title: title, addedMs: Self.nowMs())
        let audioDir = dir(book).appendingPathComponent("audio", isDirectory: true)
        try FileManager.default.createDirectory(at: audioDir, withIntermediateDirectories: true)
        var tracks: [LibraryAudioTrack] = []
        for (i, url) in urls.enumerated() {
            let name = String(format: "teil_%03d.%@", i + 1, url.pathExtension.lowercased())
            let target = audioDir.appendingPathComponent(name)
            try? FileManager.default.removeItem(at: target)
            try access(url) { try FileManager.default.copyItem(at: url, to: target) }
            let seconds = try? await AVURLAsset(url: target).load(.duration).seconds
            tracks.append(LibraryAudioTrack(file: name, title: url.deletingPathExtension().lastPathComponent,
                                            durationMs: seconds.flatMap { $0.isFinite ? $0 * 1000 : nil }))
        }
        let total = tracks.allSatisfy { $0.durationMs != nil } ? tracks.reduce(0) { $0 + ($1.durationMs ?? 0) } : nil
        book.audio = LibraryAudioVersion(file: tracks.first?.file ?? "", durationMs: total, tracks: tracks.count > 1 ? tracks : [],
                                         sync: book.audio?.sync ?? [])
        if tracks.count == 1 { book.audio?.durationMs = tracks[0].durationMs }
        try write(book)
    }

    private func match(title: String) -> LibraryBook? {
        let key = Self.normalized(title)
        return books.first { Self.normalized($0.title) == key }
    }

    static func normalized(_ s: String) -> String {
        s.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: Locale(identifier: "de"))
            .components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// Gemeinsamer Titel mehrerer Teile: längster gemeinsamer Anfang ohne Teilnummer.
    static func commonTitle(_ urls: [URL]) -> String {
        let names = urls.map { $0.deletingPathExtension().lastPathComponent }
        guard var prefix = names.first else { return "Hörbuch" }
        for n in names.dropFirst() { while !n.hasPrefix(prefix) { prefix.removeLast() } }
        let cleaned = prefix.replacingOccurrences(of: #"[\s\-_.(]*(teil|part|cd|track|kapitel)?[\s\-_.]*\d*$"#, with: "",
                                                  options: [.regularExpression, .caseInsensitive]).trimmingCharacters(in: .whitespaces)
        return cleaned.count >= 3 ? cleaned : (names.count == 1 ? names[0] : "Hörbuch")
    }

    static func nowMs() -> Double { (Date().timeIntervalSince1970 * 1000).rounded() }
    static func newId() -> String { "book_\(Int(nowMs()))" }
}
