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
    /// Für die Legende im Regal; wird beim Neuladen bestimmt, nicht bei jedem Zeichnen.
    @Published private(set) var anyAudioDownloaded = false
    @Published private(set) var progress: [String: String] = [:]
    @Published private(set) var importing = false
    @Published var error: String?
    var coverSearchRunning = false

    let root: URL
    private var saveTask: Task<Void, Never>?

    init(root: URL? = nil) {
        self.root = root ?? FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("Library")
        try? FileManager.default.createDirectory(at: self.root, withIntermediateDirectories: true,
                                                 attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        reload()
        // Nachrüstung: ältere Stände legten Hörbücher ins iCloud-Backup.
        for book in books { Self.excludeFromBackup(audioDir(book)) }
    }

    /// Hörbücher gehören nicht ins iCloud-Backup: sie liegen im Konto oder
    /// stammen vom Nutzer und machen das Backup sonst mehrere GB groß.
    /// Gilt für den Ordner und alles darin.
    static func excludeFromBackup(_ url: URL) {
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        var values = URLResourceValues(); values.isExcludedFromBackup = true
        var target = url
        try? target.setResourceValues(values)
    }

    // MARK: Lesen

    func reload() {
        // Ausstehenden Fortschritt erst sichern, sonst überschreibt das Neuladen ihn.
        if saveTask != nil { flush() }
        let fm = FileManager.default
        let dirs = (try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: nil)) ?? []
        var list: [LibraryBook] = []
        for dir in dirs where dir.hasDirectoryPath || (try? dir.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true {
            guard let data = try? Data(contentsOf: dir.appendingPathComponent("buch.json")),
                  let book = try? JSONDecoder().decode(LibraryBook.self, from: data) else { continue }
            list.append(book)
        }
        books = list.sorted { $0.addedMs > $1.addedMs }
        anyAudioDownloaded = books.contains(where: hasAudio)
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
    func audioDir(_ book: LibraryBook) -> URL { dir(book).appendingPathComponent(AudioOffload.audioFolder, isDirectory: true) }

    /// Je Teil in Hörreihenfolge die Größe der Datei auf dem iPhone, `nil` wenn sie fehlt.
    func localAudioSizes(_ book: LibraryBook) -> [Int?] {
        (book.audio?.allTracks ?? []).enumerated().map { i, track in
            LibrarySync.fileSize(audioDir(book).appendingPathComponent(LibrarySync.audioFileName(track.file, index: i)))
        }
    }

    /// Liegt überhaupt etwas vom Hörbuch auf dem iPhone? Nur ein Verzeichnisblick, billig genug für Menüs.
    func hasLocalAudio(_ book: LibraryBook) -> Bool { !AudioOffload.audioItems(in: dir(book)).isEmpty }

    /// Belegter Platz durch das Hörbuch auf dem iPhone (mit unfertigen Downloads).
    func localAudioBytes(_ book: LibraryBook) -> Int {
        AudioOffload.audioItems(in: dir(book)).reduce(0) { $0 + AudioOffload.bytes(at: $1) }
    }

    /// Freier Speicher für wichtige Daten (wie ihn iOS beim Laden berücksichtigt).
    func freeBytes() -> Int64? {
        try? root.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]).volumeAvailableCapacityForImportantUsage
    }

    /// Hörbuch vollständig auf dem iPhone, und zwar in der Fassung, die das Konto nennt.
    func hasAudio(_ book: LibraryBook) -> Bool {
        guard let tracks = book.audio?.allTracks, !tracks.isEmpty else { return false }
        let m = meta(book)
        guard !m.files.audio.isEmpty else {  // nur lokal, noch nicht im Konto
            let folder = dir(book).appendingPathComponent("audio")
            return tracks.allSatisfy { FileManager.default.fileExists(atPath: folder.appendingPathComponent($0.file).path) }
        }
        let parts = audioParts(m.files.audio)
        return AudioDownload.isCurrent(parts, sizes: parts.map { LibrarySync.fileSize(audioDir(book).appendingPathComponent($0.file)) }, blobs: m.localAudio ?? [:])
    }

    // MARK: Kurzbücher vom PC

    func kurzbuchFile(_ book: LibraryBook, _ ref: BlobRef) -> URL {
        dir(book).appendingPathComponent("kurzbuch", isDirectory: true).appendingPathComponent(ref.blob + ".md")
    }

    /// Kurzbuch als eigener Eintrag für den Player. Er steht nicht im Regal und hat seinen eigenen Lesestand.
    func kurzbuchBook(_ book: LibraryBook, _ ref: BlobRef) -> LibraryBook? {
        guard let language = LibraryText.kurzbuchLanguage(ref.file),
              let markdown = try? String(contentsOf: kurzbuchFile(book, ref), encoding: .utf8) else { return nil }
        let entry = LibraryBook(id: "\(book.id)_kurzbuch_\(language)", title: "\(book.title) · Kurzbuch", addedMs: book.addedMs)
        try? FileManager.default.createDirectory(at: dir(entry), withIntermediateDirectories: true)
        try? LibraryText.kurzbuchText(markdown).write(to: textURL(entry), atomically: true, encoding: .utf8)
        return entry
    }

    func audioParts(_ refs: [BlobRef]) -> [AudioDownload.Part] {
        refs.enumerated().map { i, ref in
            AudioDownload.Part(blob: ref.blob, size: ref.size, chunks: ref.chunks, file: LibrarySync.audioFileName(ref.file, index: i))
        }
    }

    /// Geladene Fassung einer Hörbuchdatei merken.
    func noteLocalAudio(bookId: String, file: String, blob: String) {
        guard let book = books.first(where: { $0.id == bookId }) else { return }
        var m = meta(book)
        m.localAudio = (m.localAudio ?? [:]).merging([file: blob]) { $1 }
        setMeta(book, m)
    }

    /// Dateien entfernen, die das Konto inzwischen ersetzt hat, damit sie neu geladen werden.
    func removeStaleAudio(_ book: LibraryBook) {
        let m = meta(book)
        for part in audioParts(m.files.audio) {
            let file = audioDir(book).appendingPathComponent(part.file)
            guard let size = LibrarySync.fileSize(file) else { continue }
            if !AudioDownload.isCurrent([part], sizes: [size], blobs: m.localAudio ?? [:]) { try? FileManager.default.removeItem(at: file) }
        }
    }

    // MARK: Sync-Metadaten je Buch (`<id>/sync.json`): Dateiverweise im Konto, Stand.

    struct SyncMeta: Codable {
        var files = BookFiles()
        var updated: Double = 0
        /// Dateiname → Blob der geladenen Hörbuchdatei.
        var localAudio: [String: String]? = nil
        /// Noch nicht gesendeter Stern. Beim Holen bleibt er, der Rest kommt aus dem Konto.
        var pendingFavorite: Bool? = nil
        /// Angaben ohne Stern, als der Stern gesetzt wurde; nur bei abgeglichenem Stand.
        var pendingFields: String? = nil
    }

    func meta(_ book: LibraryBook) -> SyncMeta {
        guard let data = try? Data(contentsOf: dir(book).appendingPathComponent("sync.json")),
              let m = try? JSONDecoder().decode(SyncMeta.self, from: data) else { return SyncMeta(updated: book.addedMs) }
        return m
    }

    func setMeta(_ book: LibraryBook, _ meta: SyncMeta) {
        try? FileManager.default.createDirectory(at: dir(book), withIntermediateDirectories: true)
        try? JSONEncoder().encode(meta).write(to: dir(book).appendingPathComponent("sync.json"), options: .atomic)
    }

    /// Eigene Änderung am Buch: Stand für den Abgleich erhöhen.
    func touch(_ book: LibraryBook) {
        var m = meta(book); m.updated = Self.nowMs(); setMeta(book, m)
    }

    /// Hörbuch liegt im Konto, aber (noch) nicht auf dem iPhone.
    func audioInAccount(_ book: LibraryBook) -> Bool { !meta(book).files.audio.isEmpty && !hasAudio(book) }

    /// Buch aus dem Abgleich übernehmen (ohne eigenen Stand zu erhöhen).
    func storeRemote(_ book: LibraryBook, meta: SyncMeta) throws {
        try write(book)
        var merged = meta
        merged.localAudio = self.meta(book).localAudio  // Lokale Angabe, kommt nicht aus dem Konto.
        setMeta(book, merged)
    }

    func removeRemote(id: String) {
        guard let book = books.first(where: { $0.id == id }) else { return }
        for ref in meta(book).files.kurzbuecher {
            if let language = LibraryText.kurzbuchLanguage(ref.file) { try? FileManager.default.removeItem(at: root.appendingPathComponent("\(id)_kurzbuch_\(language)")) }
        }
        try? FileManager.default.removeItem(at: dir(book))
        BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clear(bookId: book.id)
        for key in [textKey(book), audioKey(book)] { progress[Self.storageKey(key)] = nil }
        writeProgress()
        reload()
    }

    /// Fortschritt aus dem Abgleich: mit dem eigenen Stand zusammenführen.
    func mergeRemote(_ key: String, _ remote: ProgressRecord) {
        let merged = record(key).map { LibraryProgress.merge($0, remote) } ?? remote
        put(key, merged)
    }

    /// Alle Fortschrittsschlüssel ohne `progress.`-Präfix.
    var progressKeys: [String] { progress.keys.filter { $0.hasPrefix("progress.") }.map { String($0.dropFirst("progress.".count)) } }

    /// Schlüssel in `fortschritt.json` wie am Desktop (`progress.<schlüssel>`, progress.ts).
    static func storageKey(_ key: String) -> String { "progress." + key }

    func record(_ key: String) -> ProgressRecord? {
        guard let raw = progress[Self.storageKey(key)] else { return nil }
        return try? JSONDecoder().decode(ProgressRecord.self, from: Data(raw.utf8))
    }

    func textKey(_ book: LibraryBook) -> String { LibraryProgress.textKey(pageId: book.text?.pageId ?? book.id) }
    func audioKey(_ book: LibraryBook) -> String { LibraryProgress.audioKey(bookId: book.id) }

    // MARK: Schreiben

    /// Eigener Stand dieses Geräts: überschreibt wie `saveProgress` am Desktop.
    /// Zusammengeführt wird nur beim Abgleich mit anderen Geräten (Sync).
    func put(_ key: String, _ rec: ProgressRecord) {
        guard let data = try? JSONEncoder().encode(rec), let text = String(data: data, encoding: .utf8) else { return }
        progress[Self.storageKey(key)] = text
        scheduleSave()
    }

    func setFinished(_ book: LibraryBook, _ finished: Bool) {
        var copy = book; copy.finished = finished
        try? write(copy)
        touch(copy)
        if !finished {
            for key in [textKey(book), audioKey(book)] {
                guard var rec = record(key), rec.finished == true else { continue }
                rec.finished = false; rec.updated = Self.nowMs()
                put(key, rec)
            }
        }
        reload()
    }

    func setFavorite(_ book: LibraryBook, _ favorite: Bool, pendingFields: String?) {
        var copy = book; copy.favorite = favorite
        try? write(copy)
        var m = meta(copy)
        if m.pendingFavorite == nil { m.pendingFields = pendingFields }  // Zweiter Tipp: Der erste Stand gilt.
        m.updated = Self.nowMs(); m.pendingFavorite = favorite
        setMeta(copy, m)
    }

    func clearPendingFavorite(bookId: String) {
        guard let book = books.first(where: { $0.id == bookId }) else { return }
        var m = meta(book); m.pendingFavorite = nil; m.pendingFields = nil; setMeta(book, m)
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
        // Sofort sichtbar, damit ein zweites Format im selben Import das Buch findet.
        if let i = books.firstIndex(where: { $0.id == book.id }) { books[i] = book } else { books.insert(book, at: 0) }
    }

    func delete(_ book: LibraryBook) {
        try? FileManager.default.removeItem(at: dir(book))
        BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clear(bookId: book.id)
        for key in [textKey(book), audioKey(book)] { progress[Self.storageKey(key)] = nil }
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
        let data: Data = try access(url) { try Data(contentsOf: url) }
        // Entpacken und Text gewinnen abseits des Haupt-Threads.
        let extracted: ExtractedBook = try await Task.detached(priority: .userInitiated) { () throws -> ExtractedBook in
            switch ext {
            case "epub": return try BookText.epub(data)
            case "pdf":
                guard let pdf = PDFDocument(data: data) else { throw BookTextError.corrupt("PDF") }
                let pages = (0..<pdf.pageCount).compactMap { pdf.page(at: $0)?.string?.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
                guard !pages.isEmpty else { throw BookTextError.empty }
                let attrs = pdf.documentAttributes
                // Erste Seite als Titelbild; PDFs tragen kein eigenes Cover.
                let first = pdf.page(at: 0)?.thumbnail(of: CGSize(width: 900, height: 1300), for: .mediaBox).jpegData(compressionQuality: 0.85)
                return ExtractedBook(title: attrs?[PDFDocumentAttribute.titleAttribute] as? String,
                                     authors: (attrs?[PDFDocumentAttribute.authorAttribute] as? String).map { [$0] } ?? [], language: nil,
                                     text: pages.joined(separator: "\n\n"), cover: first)
            default: return ExtractedBook(title: nil, authors: [], language: nil, text: try BookText.plain(data))
            }
        }.value
        let title = (extracted.title?.isEmpty == false ? extracted.title! : url.deletingPathExtension().lastPathComponent)
        var book = match(title: title) ?? LibraryBook(id: Self.newId(), title: title, authors: extracted.authors,
                                                       coverSource: "generated", addedMs: Self.nowMs())
        if book.authors.isEmpty { book.authors = extracted.authors }
        if book.language == nil { book.language = extracted.language }
        book.text = LibraryTextVersion(pageId: "ios_\(book.id)", origin: "document", sourceName: url.lastPathComponent)
        // Cover aus dem EPUB wie am Desktop; ein vorhandenes echtes Cover bleibt.
        var newCover = false
        if let cover = extracted.cover, coverURL(book) == nil || book.coverSource == "generated" {
            try write(book)
            storeCover(cover, for: book)
            if coverURL(book) != nil { book.coverSource = "file"; newCover = true }
        }
        try write(book)
        try extracted.text.write(to: textURL(book), atomically: true, encoding: .utf8)
        var m = meta(book); m.files.text = nil; if newCover { m.files.cover = nil }; m.updated = Self.nowMs(); setMeta(book, m)
    }

    private func importAudio(_ urls: [URL]) async throws {
        let title = Self.commonTitle(urls)
        var book = match(title: title) ?? LibraryBook(id: Self.newId(), title: title, addedMs: Self.nowMs())
        try FileManager.default.createDirectory(at: dir(book), withIntermediateDirectories: true)
        let audioDir = dir(book).appendingPathComponent("audio", isDirectory: true)
        // Erst vollständig in einen neuen Ordner kopieren, dann tauschen: ein
        // Abbruch lässt das bisherige Hörbuch unverändert.
        let staging = dir(book).appendingPathComponent("audio.neu-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: staging) }
        var tracks: [LibraryAudioTrack] = []
        for (i, url) in urls.enumerated() {
            let name = String(format: "teil_%03d.%@", i + 1, url.pathExtension.lowercased())
            let target = staging.appendingPathComponent(name)
            try await Task.detached(priority: .userInitiated) {
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                try FileManager.default.copyItem(at: url, to: target)
            }.value
            let seconds = try? await AVURLAsset(url: target).load(.duration).seconds
            tracks.append(LibraryAudioTrack(file: name, title: url.deletingPathExtension().lastPathComponent,
                                            durationMs: seconds.flatMap { $0.isFinite ? $0 * 1000 : nil }))
        }
        // Durations wurden aus der Kopie gelesen; die Namen bleiben beim Tausch gleich.
        if FileManager.default.fileExists(atPath: audioDir.path) {
            _ = try FileManager.default.replaceItemAt(audioDir, withItemAt: staging)
        } else {
            try FileManager.default.moveItem(at: staging, to: audioDir)
        }
        Self.excludeFromBackup(audioDir)
        // Angaben aus den Dateien (M4B/M4A-Atome, ID3): Titel, Personen, Cover, Kapitel.
        var fileInfos: [AudiobookFileInfo] = []
        var artwork: Data?
        for t in tracks {
            let r = await AudiobookMetadataReader.read(audioDir.appendingPathComponent(t.file))
            fileInfos.append(r.file)
            if artwork == nil { artwork = r.artwork }
        }
        let found = AudiobookInfo.combine(fileInfos.map(\.info))
        let isNew = !books.contains { $0.id == book.id }
        // Ein Teil: Titel-Atom ist der Buchtitel; mehrere Teile: Album ist der Buchtitel.
        // Ohne Album bleibt bei mehreren Teilen der gemeinsame Dateiname (Teiltitel wäre „Kapitel 1“).
        let fileTitle = tracks.count > 1 ? found.album : (found.title ?? found.album)
        // Nur bei neuen Einträgen; das Audio liegt schon im Ordner dieses Eintrags.
        if isNew, let fileTitle, fileTitle.count >= 2 { book.title = fileTitle }
        if book.authors.isEmpty { book.authors = found.authors }
        if book.subtitle == nil { book.subtitle = found.subtitle }
        if book.publisher == nil { book.publisher = found.publisher }
        if book.publishDate == nil { book.publishDate = found.releaseDate }
        if book.description == nil { book.description = found.summary }
        if book.language == nil { book.language = found.language }
        let total = tracks.allSatisfy { $0.durationMs != nil } ? tracks.reduce(0) { $0 + ($1.durationMs ?? 0) } : nil
        // Neue Audiodateien: alte Zeitachse und alter Hörstand passen nicht mehr.
        let replaced = book.audio != nil
        book.audio = LibraryAudioVersion(file: tracks.first?.file ?? "", durationMs: total, tracks: tracks.count > 1 ? tracks : [], sync: [])
        if replaced { progress[Self.storageKey(audioKey(book))] = nil; scheduleSave() }
        if tracks.count == 1 { book.audio?.durationMs = tracks[0].durationMs }
        if !found.narrators.isEmpty { book.audio?.narrator = found.narrators.joined(separator: ", ") }
        try write(book)
        if let artwork, coverURL(book) == nil || book.coverSource == "generated" {
            storeCover(artwork, for: book)
            if coverURL(book) != nil {
                book.coverSource = "file"; try write(book)
                var m = meta(book); m.files.cover = nil; setMeta(book, m)  // neues Cover hochladen
            }
        }
        var info = found
        info.chapters = AudiobookChapters.merge(files: fileInfos, tracks: book.audio?.allTracks ?? tracks)
        info.chaptersFromFile = fileInfos.contains { !$0.chapters.isEmpty }
        saveAudiobookInfo(info, for: book)
        var m = meta(book); m.files.audio = []; m.updated = Self.nowMs(); setMeta(book, m)
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
    /// `book_<ms>` wie am Desktop, mit Zufallsanhang gegen Kollisionen im selben Import.
    static func newId() -> String { "book_\(Int(nowMs()))_\(UUID().uuidString.prefix(6).lowercased())" }
}
