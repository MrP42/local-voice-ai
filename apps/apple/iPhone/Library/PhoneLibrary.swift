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
    /// Text des Buchs bzw. der Sicht: `text.txt` gehört der Hauptfassung, jede weitere Fassung hat ihren Text unter
    /// `editions/<fassung>/text.txt` (Sprachfassungen lva#344). Nur Sichten (`editionId`) lesen die Fassungen.
    func textURL(_ book: LibraryBook) -> URL {
        if let id = book.editionId, Self.isSafeEditionId(id), id != flatEditionId(book) {
            return editionTextURL(book, id)
        }
        return dir(book).appendingPathComponent("text.txt")
    }
    /// Fassung, der die flachen Dateien (`text.txt`, `files.text/cover/audio`) gehören: die Hauptfassung des Senders
    /// (`sync.json`), ohne Vermerk die des Buchs.
    func flatEditionId(_ book: LibraryBook, meta known: SyncMeta? = nil) -> String? {
        (known ?? meta(book)).flatEdition ?? BookEditions(book: book).primaryId
    }
    func editionTextURL(_ book: LibraryBook, _ editionId: String) -> URL {
        dir(book).appendingPathComponent("editions", isDirectory: true).appendingPathComponent(editionId, isDirectory: true)
            .appendingPathComponent("text.txt")
    }
    /// Kennung kommt vom Netz und landet in Pfaden: wie `valid_edition_id` im Kern.
    static func isSafeEditionId(_ id: String) -> Bool {
        id.hasPrefix("ed_") && id.utf8.count <= 80 && id.utf8.allSatisfy {
            ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x41 && $0 <= 0x5a) || ($0 >= 0x61 && $0 <= 0x7a) || $0 == 0x5f || $0 == 0x2d
        }
    }
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
        let refs = accountAudio(book, meta: m)
        guard !refs.isEmpty else {  // nur lokal, noch nicht im Konto
            let folder = dir(book).appendingPathComponent("audio")
            return tracks.allSatisfy { FileManager.default.fileExists(atPath: folder.appendingPathComponent($0.file).path) }
        }
        let parts = audioParts(refs)
        return AudioDownload.isCurrent(parts, sizes: parts.map { LibrarySync.fileSize(audioDir(book).appendingPathComponent($0.file)) }, blobs: m.localAudio ?? [:])
    }

    /// Hörbuchteile dieses Buchs bzw. dieser Sicht laut Konto: Die flachen `files.audio` gehören der Hauptfassung
    /// des Senders, weitere Fassungen stehen in `files.editions[<fassung>].audio`.
    func accountAudio(_ book: LibraryBook, meta known: SyncMeta? = nil) -> [BlobRef] {
        let m = known ?? meta(book)
        guard let id = book.editionId else { return m.files.audio }
        return m.files.files(forEdition: id, flat: flatEditionId(book, meta: m)).audio
    }

    // MARK: Kurzbücher vom PC

    func kurzbuecher(_ book: LibraryBook) -> [KurzbuchRef] { meta(book).files.kurzbuecher.filter(\.isUsable) }

    func kurzbuchFile(_ book: LibraryBook, _ ref: KurzbuchRef) -> URL? {
        guard ref.isUsable else { return nil }
        return dir(book).appendingPathComponent("kurzbuch", isDirectory: true).appendingPathComponent(ref.blob + ".md")
    }

    /// Geladen und nicht leer.
    func kurzbuchReady(_ book: LibraryBook, _ ref: KurzbuchRef) -> Bool {
        kurzbuchFile(book, ref).flatMap(LibrarySync.fileSize) == ref.size
    }

    /// Mit Fassung im Namen: Ein neu erzeugtes Kurzbuch beginnt mit eigenem Lesestand.
    private func kurzbuchEntryId(_ bookId: String, _ ref: KurzbuchRef, _ language: String) -> String {
        "\(bookId)_kurzbuch_\(language)_\(ref.blob.prefix(12))"
    }

    /// Kurzbuch als eigener Eintrag für den Player. Er steht nicht im Regal und hat seinen eigenen Lesestand.
    func kurzbuchBook(_ book: LibraryBook, _ ref: KurzbuchRef) -> LibraryBook? {
        guard let language = ref.language, kurzbuchReady(book, ref), let file = kurzbuchFile(book, ref),
              let markdown = try? String(contentsOf: file, encoding: .utf8),
              !markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        var entry = LibraryBook(id: kurzbuchEntryId(book.id, ref, language), title: "\(book.title) · Kurzbuch", addedMs: book.addedMs)
        // Sprache des Kurzbuchs: wählt die Vorlesestimme (lva#352).
        entry.language = language
        // Eine echte Buch-Kennung könnte zufällig gleich aussehen: nie in deren Ordner schreiben.
        guard !isRealBookFolder(dir(entry)) else { return nil }
        try? FileManager.default.createDirectory(at: dir(entry), withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: dir(entry).appendingPathComponent(Self.kurzbuchMarker).path, contents: Data())
        try? LibraryText.kurzbuchText(markdown).write(to: textURL(entry), atomically: true, encoding: .utf8)
        return entry
    }

    /// Alte Fassungen, entfernte Sprachen und deren Klartextkopien samt Lesestand aufräumen.
    func pruneKurzbuecher(_ book: LibraryBook) {
        let refs = kurzbuecher(book)
        let manager = FileManager.default
        let folder = dir(book).appendingPathComponent("kurzbuch", isDirectory: true)
        let keep = Set(refs.map { $0.blob + ".md" })
        for name in (try? manager.contentsOfDirectory(atPath: folder.path)) ?? [] where !keep.contains(name) {
            try? manager.removeItem(at: folder.appendingPathComponent(name))
        }
        removeKurzbuchEntries(bookId: book.id, keeping: Set(refs.compactMap { ref in ref.language.map { kurzbuchEntryId(book.id, ref, $0) } }))
    }

    /// Kennzeichnet einen Ordner als Kurzbuch-Eintrag.
    private static let kurzbuchMarker = "kurzbuch-eintrag"

    private func isRealBookFolder(_ folder: URL) -> Bool {
        FileManager.default.fileExists(atPath: folder.appendingPathComponent("buch.json").path)
    }

    private func removeKurzbuchEntries(bookId: String, keeping: Set<String> = []) {
        let prefix = "\(bookId)_kurzbuch_"
        var changed = false
        for name in (try? FileManager.default.contentsOfDirectory(atPath: root.path)) ?? [] where name.hasPrefix(prefix) && !keeping.contains(name) {
            let folder = root.appendingPathComponent(name)
            // Nur eigene Einträge: Der Name allein beweist nichts, ein Buch könnte so heißen.
            guard FileManager.default.fileExists(atPath: folder.appendingPathComponent(Self.kurzbuchMarker).path), !isRealBookFolder(folder) else { continue }
            if LibraryPlayback.shared.book?.id == name { LibraryPlayback.shared.stop() }
            try? FileManager.default.removeItem(at: root.appendingPathComponent(name))
            progress[Self.storageKey(LibraryProgress.textKey(pageId: name))] = nil
            BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clear(bookId: name)
            changed = true
        }
        if changed { writeProgress() }
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
        for part in audioParts(accountAudio(book, meta: m)) {
            let file = audioDir(book).appendingPathComponent(part.file)
            guard let size = LibrarySync.fileSize(file) else { continue }
            if !AudioDownload.isCurrent([part], sizes: [size], blobs: m.localAudio ?? [:]) { try? FileManager.default.removeItem(at: file) }
        }
    }

    // MARK: Sync-Metadaten je Buch (`<id>/sync.json`): Dateiverweise im Konto, Stand.

    typealias SyncMeta = LibrarySyncMeta

    func meta(_ book: LibraryBook) -> SyncMeta {
        guard let data = try? Data(contentsOf: dir(book).appendingPathComponent("sync.json")),
              let m = try? JSONDecoder().decode(SyncMeta.self, from: data) else { return SyncMeta(updated: book.addedMs) }
        return m
    }

    /// Dateiverweis nach einem Upload eintragen. Liest frisch, denn während des Uploads kann ein Stern dazugekommen sein.
    func updateFiles(_ book: LibraryBook, _ change: (inout BookFiles) -> Void) {
        setMeta(book, meta(book).updatingFiles(change))
    }

    func setMeta(_ book: LibraryBook, _ meta: SyncMeta) {
        try? FileManager.default.createDirectory(at: dir(book), withIntermediateDirectories: true)
        try? JSONEncoder().encode(meta).write(to: dir(book).appendingPathComponent("sync.json"), options: .atomic)
    }

    /// Eigene Änderung am Buch: Stand für den Abgleich erhöhen.
    func touch(_ book: LibraryBook) {
        var m = meta(book); m.updated = Self.nowMs(); setMeta(book, m)
    }

    // MARK: Sprachfassungen (lva#344)

    /// Stabile Kennung dieses iPhones für die Register (`device`), nur lokal gespeichert.
    static var deviceTag: String {
        let key = "lva.library.device"
        if let stored = UserDefaults.standard.string(forKey: key), !stored.isEmpty { return stored }
        let fresh = "ios_" + UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(8).lowercased()
        UserDefaults.standard.set(fresh, forKey: key)
        return fresh
    }

    /// Das Buch, wie es im Regal steht (nie eine Sicht).
    func rawBook(_ id: String) -> LibraryBook? { books.first { $0.id == id } }

    /// Das Buch aus Sicht einer Fassung: Titel, Untertitel, Sprache, Text und Hörbuch der Fassung statt der
    /// Hauptfassung. Kennung, Ordner und Konto bleiben die des Buchs. Alle Zugriffe auf Text, Hörbuch und
    /// Fortschritt (`textURL`, `hasAudio`, `audioKey` …) richten sich nach `editionId`.
    func editionView(_ book: LibraryBook, edition e: BookEdition) -> LibraryBook {
        var view = book
        view.editionId = e.id
        view.title = e.title.trimmingCharacters(in: .whitespaces).isEmpty ? book.title : e.title
        view.subtitle = e.subtitle
        if !e.lang.isEmpty { view.language = e.lang }
        view.text = e.text
        view.audio = e.audio
        return view
    }

    /// Sicht auf die Fassung `editionId`; ohne Sprachfassungen oder bei unbekannter Kennung das Buch selbst.
    func view(_ book: LibraryBook, editionId: String?) -> LibraryBook {
        guard let editionId, book.editions != nil else { return book }
        let editions = BookEditions(book: book)
        guard !editions.synthesized, editions.hasLanguages, let e = editions.edition(editionId) else { return book }
        return editionView(book, edition: e)
    }

    /// Sicht auf die zuletzt aktive Fassung (`reading`, sonst Hauptfassung): für Regal und Buchseite. Ohne
    /// Sprachfassungen ist es das Buch selbst. Billig, solange das Buch nur eine Fassung hat.
    func activeView(_ book: LibraryBook) -> LibraryBook {
        guard (book.editions?.arrayValue?.count ?? 0) > 1 else { return book }
        let editions = BookEditions(book: book)
        guard editions.hasLanguages, let e = editions.readingEdition else { return book }
        return editionView(book, edition: e)
    }

    /// Liegt etwas zum Hören oder Lesen auf dem iPhone?
    func isPlayable(_ view: LibraryBook) -> Bool { hasText(view) || hasAudio(view) }

    /// Wie `activeView`, aber nur eine Fassung, die sich auf diesem iPhone abspielen lässt: Hat der PC die
    /// Auswahl auf eine noch nicht geladene Fassung gesetzt, öffnet der Player die erste, die da ist.
    func playableView(_ book: LibraryBook) -> LibraryBook {
        let preferred = activeView(book)
        guard preferred.editionId != nil, !isPlayable(preferred) else { return preferred }
        let editions = BookEditions(book: book)
        var order = editions.editions
        if let primary = editions.primary { order.insert(primary, at: 0) }
        for e in order {
            let candidate = editionView(book, edition: e)
            if isPlayable(candidate) { return candidate }
        }
        return preferred
    }

    /// Stelle der Zielfassung in den Fortschritt, dann die Auswahl (`reading`) ins Buch schreiben und das Buch
    /// für den nächsten Abgleich vormerken. Der Fortschritt läuft je Fassung unter `edition.audio_key` bzw.
    /// `text:<seite>:original`, nie unter der Buchkennung. Liefert das Buch (Regal-Stand) oder `nil`, wenn die
    /// Fassung nicht lebt. Gehörte und gelesene Bereiche bleiben; die Stelle zählt nicht als gehört.
    @discardableResult
    func writeSwitch(bookId: String, plan: SwitchPlan, length: Double) -> LibraryBook? {
        guard let raw = rawBook(bookId) else { return nil }
        let now = Self.nowMs()
        guard let changed = raw.settingReading(edition: plan.to, position: plan.position, device: Self.deviceTag, now: now),
              (try? write(changed)) != nil else { return nil }
        var rec = record(plan.position.key) ?? ProgressRecord(offset: 0, length: 0, updated: 0)
        rec.offset = plan.position.offset
        if length > 0 { rec.length = length }
        rec.unit = plan.toMedium == .audio ? .ms : .chars
        rec.anchor = nil
        rec.finished = nil  // eine als fertig markierte Zielfassung beginnt an der Zielstelle
        rec.updated = now
        put(plan.position.key, rec)
        touch(changed)
        return changed
    }

    /// Stand, den ein Wechsel überschrieben hat, für „Rückgängig“.
    struct SwitchUndo {
        let bookId: String
        let fromEdition: String
        /// Stelle der Quellfassung zum Zeitpunkt des Wechsels.
        let fromPosition: ReadingPositionRef?
        /// Fortschrittsdatensatz der Zielfassung vor dem Wechsel.
        let targetKey: String
        let targetRecord: ProgressRecord?
        /// Spielte es beim Wechsel? Rückgängig startet genauso.
        let wasPlaying: Bool
    }

    /// Rückgängig: Auswahl zurück auf die Quellfassung samt Stelle. Was in der Zielfassung seit dem Wechsel
    /// gehört wurde, bleibt (Vereinigung der Bereiche); Stelle und „fertig“ gelten wie vor dem Wechsel. Datensätze
    /// sind „jetzt“ geschrieben, sonst überstimmte der Wechselstand sie beim nächsten Abgleich.
    @discardableResult
    func undoSwitch(_ undo: SwitchUndo) -> LibraryBook? {
        guard let raw = rawBook(undo.bookId) else { return nil }
        let now = Self.nowMs()
        guard let changed = raw.settingReading(edition: undo.fromEdition, position: undo.fromPosition, device: Self.deviceTag, now: now),
              (try? write(changed)) != nil else { return nil }
        if let restored = LibraryProgress.undoingSwitch(current: record(undo.targetKey), before: undo.targetRecord, now: now) {
            put(undo.targetKey, restored)
        } else {
            removeProgress(undo.targetKey)
        }
        if let from = undo.fromPosition, var source = record(from.key) {
            source.offset = from.offset
            source.updated = now
            put(from.key, source)
        }
        touch(changed)
        return changed
    }

    func removeProgress(_ key: String) {
        guard progress[Self.storageKey(key)] != nil else { return }
        progress[Self.storageKey(key)] = nil
        scheduleSave()
    }

    // MARK: Text weiterer Fassungen

    private func editionTextBlobURL(_ book: LibraryBook, _ editionId: String) -> URL {
        editionTextURL(book, editionId).deletingLastPathComponent().appendingPathComponent("text.blob")
    }

    /// Welcher Blob liegt als Text dieser (weiteren) Fassung auf dem iPhone?
    func editionTextBlob(_ book: LibraryBook, _ editionId: String) -> String? {
        guard Self.isSafeEditionId(editionId), FileManager.default.fileExists(atPath: editionTextURL(book, editionId).path) else { return nil }
        return (try? String(contentsOf: editionTextBlobURL(book, editionId), encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func noteEditionText(_ book: LibraryBook, _ editionId: String, blob: String) {
        guard Self.isSafeEditionId(editionId) else { return }
        try? blob.write(to: editionTextBlobURL(book, editionId), atomically: true, encoding: .utf8)
    }

    /// Wechselt die Hauptfassung (am PC), wandern die Texte mit: `text.txt` gehört immer der Hauptfassung, die
    /// bisherige kommt unter `editions/<fassung>/`, die neue wird zu `text.txt`. Nicht mehr lebende Fassungen
    /// verlieren ihren Ordner.
    private func adoptEditionFiles(from old: LibraryBook, to new: LibraryBook, oldFlat: String?, newFlat: String?) {
        guard old.editions != nil || new.editions != nil else { return }
        let after = BookEditions(book: new)
        let fm = FileManager.default
        let folder = dir(new)
        if let a = oldFlat, let b = newFlat, a != b, Self.isSafeEditionId(a), Self.isSafeEditionId(b) {
            let flat = folder.appendingPathComponent("text.txt")
            let hold = folder.appendingPathComponent("text.wechsel")
            let oldHome = editionTextURL(new, a), newHome = editionTextURL(new, b)
            try? fm.removeItem(at: hold)
            if fm.fileExists(atPath: flat.path) { try? fm.moveItem(at: flat, to: hold) }
            if fm.fileExists(atPath: newHome.path) {
                try? fm.moveItem(at: newHome, to: flat)
                try? fm.removeItem(at: editionTextBlobURL(new, b))
            }
            if fm.fileExists(atPath: hold.path) {
                try? fm.createDirectory(at: oldHome.deletingLastPathComponent(), withIntermediateDirectories: true)
                try? fm.removeItem(at: oldHome)
                try? fm.moveItem(at: hold, to: oldHome)
            }
        }
        let live = Set(after.editions.map(\.id))
        let editionsDir = folder.appendingPathComponent("editions", isDirectory: true)
        for name in (try? fm.contentsOfDirectory(atPath: editionsDir.path)) ?? [] where !live.contains(name) {
            try? fm.removeItem(at: editionsDir.appendingPathComponent(name))
        }
    }

    /// Hörbuch liegt im Konto, aber (noch) nicht auf dem iPhone.
    func audioInAccount(_ book: LibraryBook) -> Bool { !accountAudio(book).isEmpty && !hasAudio(book) }

    /// Buch aus dem Abgleich übernehmen (ohne eigenen Stand zu erhöhen).
    func storeRemote(_ book: LibraryBook, meta: SyncMeta) throws {
        let local = rawBook(book.id)
        let flat = BookEditions.senderPrimary(local: local, incoming: book)
        if let local { adoptEditionFiles(from: local, to: book, oldFlat: flatEditionId(local), newFlat: flat) }
        try write(book)
        var merged = meta
        merged.flatEdition = flat
        merged.localAudio = self.meta(book).localAudio  // Lokale Angabe, kommt nicht aus dem Konto.
        setMeta(book, merged)
    }

    func removeRemote(id: String) {
        guard let book = books.first(where: { $0.id == id }) else { return }
        removeKurzbuchEntries(bookId: id)
        try? FileManager.default.removeItem(at: dir(book))
        BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clear(bookId: book.id)
        for key in allProgressKeys(book) { progress[Self.storageKey(key)] = nil }
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

    // MARK: Stimme je Buchsprache (lva#352)

    /// Roh-JSON eines `tts.voice.*`-Eintrags (liegt wie am Desktop in `fortschritt.json`).
    func voicePrefRaw(_ key: String) -> String? { progress[key] }
    var voicePrefKeys: [String] { progress.keys.filter { $0.hasPrefix(VoicePrefs.prefix) }.sorted() }
    func setVoicePrefRaw(_ key: String, _ raw: String) {
        guard key.hasPrefix(VoicePrefs.prefix), progress[key] != raw else { return }
        progress[key] = raw
        scheduleSave()
    }

    func textKey(_ book: LibraryBook) -> String { LibraryProgress.textKey(pageId: book.text?.pageId ?? book.id) }
    /// Fortschrittsschlüssel des Hörbuchs: der der Fassung, die dieses Buch (bzw. diese Sicht) darstellt, und nie
    /// aus der Buchkennung abgeleitet, wenn Fassungen da sind (`edition.audio_key`, Spezifikation §4).
    func audioKey(_ book: LibraryBook) -> String {
        guard book.editions != nil else { return LibraryProgress.audioKey(bookId: book.id) }
        let editions = BookEditions(book: book)
        return (editions.edition(book.editionId) ?? editions.primary)?.audioKey ?? LibraryProgress.audioKey(bookId: book.id)
    }

    /// Alle Fortschrittsschlüssel des Buchs, über alle Fassungen (zum Löschen).
    func allProgressKeys(_ book: LibraryBook) -> [String] {
        var keys = [textKey(book), LibraryProgress.audioKey(bookId: book.id), audioKey(book)]
        for e in BookEditions(book: book).editions {
            keys.append(e.audioKey)
            if let t = e.textKey { keys.append(t) }
        }
        return Array(Set(keys))
    }

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
            for key in allProgressKeys(book) {
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

    func write(_ book: LibraryBook) throws {
        var book = book
        if book.editionId != nil {
            // Eine Sicht auf eine Sprachfassung gehört nie in `buch.json` (Titel, Text und Hörbuch der Hauptfassung
            // würden überschrieben): Es zählen nur die Angaben, die sich am Buch selbst ändern.
            guard var raw = rawBook(book.id) else { return }
            raw.finished = book.finished; raw.favorite = book.favorite
            book = raw
        }
        let enc = JSONEncoder(); enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(at: dir(book), withIntermediateDirectories: true)
        try enc.encode(book).write(to: dir(book).appendingPathComponent("buch.json"), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        // Sofort sichtbar, damit ein zweites Format im selben Import das Buch findet.
        if let i = books.firstIndex(where: { $0.id == book.id }) { books[i] = book } else { books.insert(book, at: 0) }
    }

    func delete(_ book: LibraryBook) {
        removeKurzbuchEntries(bookId: book.id)
        try? FileManager.default.removeItem(at: dir(book))
        BookTalkHistoryStore(directory: BookTalkHistoryStore.standardDirectory()).clear(bookId: book.id)
        for key in allProgressKeys(book) { progress[Self.storageKey(key)] = nil }
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
