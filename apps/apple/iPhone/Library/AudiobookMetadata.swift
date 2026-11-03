import Foundation
import AVFoundation
import UIKit

/// Liest Titel, Personen, Verlag, Datum, Beschreibung, Cover und Kapitel aus
/// Hörbuchdateien (M4B/M4A/AAC über die MP4-Atome, MP3 über ID3).
/// Reine Logik (Zusammenführen, Seiten, Kapitelsuche) liegt in VoiceCore/Audiobook.swift.
enum AudiobookMetadataReader {
    struct FileResult {
        var file: AudiobookFileInfo
        var artwork: Data?
    }

    static func read(_ url: URL) async -> FileResult {
        let asset = AVURLAsset(url: url)
        var info = AudiobookInfo()
        var artwork: Data?
        let ext = url.pathExtension.lowercased()
        info.format = ext.isEmpty ? nil : ext.uppercased()
        info.fileBytes = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init)

        let seconds = try? await asset.load(.duration).seconds
        let durationMs = seconds.flatMap { $0.isFinite && $0 > 0 ? $0 * 1000 : nil }

        var items: [AVMetadataItem] = []
        if let all = try? await asset.load(.metadata) { items += all }
        if let common = try? await asset.load(.commonMetadata) { items += common }

        for item in items {
            let id = item.identifier
            let raw = id?.rawValue ?? ""
            if artwork == nil, id == .commonIdentifierArtwork || id == .iTunesMetadataCoverArt || id == .id3MetadataAttachedPicture {
                artwork = try? await item.load(.dataValue)
                continue
            }
            guard let value = await string(item) else { continue }
            guard let id else { continue }
            switch id {
            case .commonIdentifierTitle, .iTunesMetadataSongName, .id3MetadataTitleDescription:
                info.title = info.title ?? value
            case .iTunesMetadataArtist, .commonIdentifierArtist, .id3MetadataLeadPerformer, .commonIdentifierAuthor,
                 .iTunesMetadataAuthor:
                info.authors += AudiobookInfo.people(value)
            case .iTunesMetadataAlbumArtist, .id3MetadataBand:
                if info.authors.isEmpty { info.authors += AudiobookInfo.people(value) }
            case .iTunesMetadataComposer, .id3MetadataComposer:
                // Hörbuch-Konvention (Audible, m4b-tool, Audiobookshelf): Sprecher im Komponistenfeld.
                info.narrators += AudiobookInfo.people(value)
            case .commonIdentifierAlbumName, .iTunesMetadataAlbum, .id3MetadataAlbumTitle:
                info.album = info.album ?? value
            case .iTunesMetadataGrouping, .id3MetadataContentGroupDescription:
                info.series = info.series ?? value
            case .commonIdentifierPublisher, .iTunesMetadataPublisher, .id3MetadataPublisher:
                info.publisher = info.publisher ?? value
            case .iTunesMetadataReleaseDate, .commonIdentifierCreationDate, .id3MetadataRecordingTime, .id3MetadataYear,
                 .id3MetadataOriginalReleaseTime, .id3MetadataReleaseTime:
                info.releaseDate = info.releaseDate ?? value
            case .iTunesMetadataUserGenre, .id3MetadataContentType, .commonIdentifierType:
                info.genre = info.genre ?? value
            case .commonIdentifierLanguage, .id3MetadataLanguage:
                info.language = info.language ?? value
            case .commonIdentifierDescription, .iTunesMetadataDescription, .iTunesMetadataUserComment, .id3MetadataComments:
                // Die längste Beschreibung gewinnt (`ldes` ist meist ausführlicher als `desc`).
                if value.count > (info.summary?.count ?? 0) { info.summary = value }
            case .commonIdentifierCopyrights, .iTunesMetadataCopyright, .id3MetadataCopyright:
                info.copyright = info.copyright ?? value
            case .iTunesMetadataEncodingTool, .iTunesMetadataEncodedBy, .commonIdentifierSoftware, .id3MetadataEncodedWith:
                info.encoder = info.encoder ?? value
            default:
                // Freiform-Atome ohne festen Bezeichner.
                let key = raw.lowercased()
                if key.hasSuffix("ldes"), value.count > (info.summary?.count ?? 0) { info.summary = value }
                else if key.hasSuffix("narrator") || key.hasSuffix("©nrt") { info.narrators += AudiobookInfo.people(value) }
                else if key.hasSuffix("subtitle") || key.hasSuffix("tit3") { info.subtitle = info.subtitle ?? value }
                else if key.hasSuffix("series") || key.hasSuffix("©mvn") { info.series = info.series ?? value }
            }
        }
        info.authors = AudiobookInfo.unique(info.authors)
        info.narrators = AudiobookInfo.unique(info.narrators)
        info.summary = info.summary.map(plainText)

        if let track = try? await asset.loadTracks(withMediaType: .audio).first {
            if let rate = try? await track.load(.estimatedDataRate), rate > 0 { info.bitrateKbps = Int((rate / 1000).rounded()) }
            if let formats = try? await track.load(.formatDescriptions), let f = formats.first {
                info.codec = codecName(CMFormatDescriptionGetMediaSubType(f))
            }
        }

        var chapters: [AudiobookChapter] = []
        let languages = Locale.preferredLanguages + ["de", "en"]
        if let groups = try? await asset.loadChapterMetadataGroups(bestMatchingPreferredLanguages: languages) {
            for (n, group) in groups.enumerated() {
                let range = group.timeRange
                guard range.start.seconds.isFinite else { continue }
                var title: String?
                for item in group.items where item.commonKey == .commonKeyTitle || item.identifier == .quickTimeUserDataChapter {
                    if let s = await string(item) { title = s; break }
                }
                let duration = range.duration.seconds.isFinite ? range.duration.seconds * 1000 : 0
                chapters.append(AudiobookChapter(title: title ?? "Kapitel \(n + 1)", startMs: range.start.seconds * 1000, durationMs: duration))
            }
        }
        if artwork != nil { info.hasCover = true }
        return FileResult(file: AudiobookFileInfo(info: info, chapters: chapters, durationMs: durationMs), artwork: artwork)
    }

    private static func string(_ item: AVMetadataItem) async -> String? {
        let s = (try? await item.load(.stringValue)) ?? nil
        if let s { let t = s.trimmingCharacters(in: .whitespacesAndNewlines); return t.isEmpty ? nil : t }
        if let d = (try? await item.load(.dateValue)) ?? nil { return ISO8601DateFormatter().string(from: d) }
        if let n = (try? await item.load(.numberValue)) ?? nil { return n.stringValue }
        return nil
    }

    /// Beschreibungen kommen teils als HTML (Audible, Verlage).
    private static func plainText(_ s: String) -> String {
        guard s.contains("<") else { return s }
        var t = s.replacingOccurrences(of: #"<br\s*/?>|</p>"#, with: "\n", options: [.regularExpression, .caseInsensitive])
        t = t.replacingOccurrences(of: #"<[^>]+>"#, with: "", options: .regularExpression)
        for (a, b) in [("&amp;", "&"), ("&quot;", "\""), ("&#39;", "'"), ("&lt;", "<"), ("&gt;", ">"), ("&nbsp;", " ")] {
            t = t.replacingOccurrences(of: a, with: b)
        }
        return t.replacingOccurrences(of: #"\n{3,}"#, with: "\n\n", options: .regularExpression).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func codecName(_ code: FourCharCode) -> String {
        switch code {
        case kAudioFormatMPEG4AAC, kAudioFormatMPEG4AAC_HE, kAudioFormatMPEG4AAC_HE_V2, kAudioFormatMPEG4AAC_LD: return "AAC"
        case kAudioFormatMPEGLayer3: return "MP3"
        case kAudioFormatAppleLossless: return "Apple Lossless"
        case kAudioFormatFLAC: return "FLAC"
        case kAudioFormatOpus: return "Opus"
        default:
            let bytes = [24, 16, 8, 0].map { UInt8((code >> $0) & 0xFF) }
            return String(bytes: bytes, encoding: .ascii)?.trimmingCharacters(in: .whitespaces) ?? "Audio"
        }
    }

    /// Cover für die Ablage: zu große Bilder verkleinern, immer als JPEG.
    static func coverJPEG(_ data: Data, maxSide: CGFloat = 1400) -> Data? {
        guard let image = UIImage(data: data) else { return nil }
        let side = max(image.size.width, image.size.height)
        guard side > maxSide else { return image.jpegData(compressionQuality: 0.88) }
        let scale = maxSide / side
        let size = CGSize(width: (image.size.width * scale).rounded(), height: (image.size.height * scale).rounded())
        let format = UIGraphicsImageRendererFormat(); format.scale = 1
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
            .jpegData(compressionQuality: 0.88)
    }
}

// MARK: - Ablage je Buch

extension PhoneLibrary {
    var coverCache: NSCache<NSString, UIImage> { PhoneLibraryCoverCache.shared }

    func coverURL(_ book: LibraryBook) -> URL? {
        ["cover.jpg", "cover.png", "cover.webp"].map { dir(book).appendingPathComponent($0) }
            .first { FileManager.default.fileExists(atPath: $0.path) }
    }

    /// Cover des Buchs, falls vorhanden (aus der Hörbuchdatei, EPUB oder vom Desktop).
    func coverImage(_ book: LibraryBook) -> UIImage? {
        guard let url = coverURL(book) else { return nil }
        let stamp = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate?.timeIntervalSince1970) ?? 0
        let key = "\(url.path)#\(stamp)" as NSString
        if let hit = coverCache.object(forKey: key) { return hit }
        guard let image = UIImage(contentsOfFile: url.path) else { return nil }
        coverCache.setObject(image, forKey: key)
        return image
    }

    func storeCover(_ data: Data, for book: LibraryBook) {
        guard let jpeg = AudiobookMetadataReader.coverJPEG(data) else { return }
        for old in ["cover.png", "cover.webp"] { try? FileManager.default.removeItem(at: dir(book).appendingPathComponent(old)) }
        try? jpeg.write(to: dir(book).appendingPathComponent("cover.jpg"), options: .atomic)
        objectWillChange.send()
    }

    private func infoURL(_ book: LibraryBook) -> URL { dir(book).appendingPathComponent("hoerbuch-info.json") }

    /// Signatur der Audiodateien; ändert sie sich, wird neu gelesen.
    private func audioSignature(_ book: LibraryBook) -> String {
        (book.audio?.allTracks ?? []).map { "\($0.file):\(Int($0.durationMs ?? 0))" }.joined(separator: "|")
    }

    func cachedAudiobookInfo(_ book: LibraryBook) -> AudiobookInfo? {
        guard let data = try? Data(contentsOf: infoURL(book)),
              let stored = try? JSONDecoder().decode(StoredAudiobookInfo.self, from: data),
              stored.signature == audioSignature(book), stored.info.version == AudiobookInfo.currentVersion else { return nil }
        return stored.info
    }

    /// Angaben und Kapitel des Hörbuchs: aus dem Cache, sonst aus den Dateien
    /// gelesen (z. B. nach dem Laden eines am PC hinzugefügten Hörbuchs).
    func audiobookInfo(_ book: LibraryBook) async -> AudiobookInfo? {
        if let cached = cachedAudiobookInfo(book) { return cached }
        guard let tracks = book.audio?.allTracks, !tracks.isEmpty, hasAudio(book) else { return nil }
        let urls = audioURLs(book)
        var files: [AudiobookFileInfo] = []
        var artwork: Data?
        for url in urls {
            let r = await AudiobookMetadataReader.read(url)
            files.append(r.file)
            if artwork == nil { artwork = r.artwork }
        }
        var info = AudiobookInfo.combine(files.map(\.info))
        info.chapters = AudiobookChapters.merge(files: files, tracks: tracks)
        info.chaptersFromFile = files.contains { !$0.chapters.isEmpty }
        if let artwork, coverURL(book) == nil { storeCover(artwork, for: book) }
        saveAudiobookInfo(info, for: book)
        return info
    }

    func saveAudiobookInfo(_ info: AudiobookInfo, for book: LibraryBook) {
        let stored = StoredAudiobookInfo(signature: audioSignature(book), info: info)
        try? JSONEncoder().encode(stored).write(to: infoURL(book), options: .atomic)
    }
}

private struct StoredAudiobookInfo: Codable {
    var signature: String
    var info: AudiobookInfo
}

private enum PhoneLibraryCoverCache {
    static let shared: NSCache<NSString, UIImage> = { let c = NSCache<NSString, UIImage>(); c.countLimit = 40; return c }()
}
