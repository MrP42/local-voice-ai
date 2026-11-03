import Foundation

// Hörbuch-Metadaten und Kapitel (Thread „iPhone-Hörbuchplayer wie Audible gestalten“).
//
// M4B/M4A (AAC im MP4-Container) bringen neben Titel und Autor meist Sprecher
// (`©wrt`/`©nrt`), Verlag, Erscheinungsdatum, Beschreibung (`desc`/`ldes`),
// Copyright, Genre, Cover (`covr`) und eine Kapitelspur mit. MP3-Hörbücher
// tragen dieselben Angaben in ID3 (`TIT2`, `TPE1`, `TCOM`, `APIC`, `CHAP`).
// Diese Datei enthält nur die reine Logik; das Auslesen mit AVFoundation
// liegt in `iPhone/Library/AudiobookMetadata.swift`.
//
// Die Kapitel werden je Gerät aus den Audiodateien gewonnen und als Cache
// (`<id>/hoerbuch-info.json`) abgelegt. `buch.json` bleibt unverändert, damit
// das mit dem Desktop geteilte Datenmodell nicht wächst.

public struct AudiobookChapter: Codable, Equatable, Sendable {
    public var title: String
    /// Beginn im gesamten Hörbuch (über alle Teile), in Millisekunden.
    public var startMs: Double
    public var durationMs: Double
    public var endMs: Double { startMs + durationMs }
    public init(title: String, startMs: Double, durationMs: Double) {
        self.title = title; self.startMs = startMs; self.durationMs = durationMs
    }
}

/// Angaben aus den Audiodateien; jedes Feld optional, weil Hörbuchdateien
/// sehr unterschiedlich gepflegt sind.
public struct AudiobookInfo: Codable, Equatable, Sendable {
    public var title: String?
    public var subtitle: String?
    public var authors: [String] = []
    public var narrators: [String] = []
    public var album: String?
    public var series: String?
    public var publisher: String?
    public var releaseDate: String?
    public var genre: String?
    public var language: String?
    public var summary: String?
    public var copyright: String?
    public var encoder: String?
    /// Containerformat, z. B. „M4B“, „MP3“.
    public var format: String?
    /// Audiocodec, z. B. „AAC“.
    public var codec: String?
    public var bitrateKbps: Int?
    public var fileBytes: Int64?
    public var chapters: [AudiobookChapter] = []
    /// `true`, wenn die Kapitel aus der Datei stammen; `false` = Teile als Kapitel.
    public var chaptersFromFile = false
    public var hasCover = false
    /// Version des Auslesers; ältere Caches werden neu gelesen.
    public var version = AudiobookInfo.currentVersion

    public static let currentVersion = 1

    public init() {}
}

/// Ergebnis für eine einzelne Audiodatei (ein Teil).
public struct AudiobookFileInfo: Equatable, Sendable {
    public var info: AudiobookInfo
    /// Kapitel relativ zum Beginn dieser Datei.
    public var chapters: [AudiobookChapter]
    public var durationMs: Double?
    public init(info: AudiobookInfo = AudiobookInfo(), chapters: [AudiobookChapter] = [], durationMs: Double? = nil) {
        self.info = info; self.chapters = chapters; self.durationMs = durationMs
    }
}

public enum AudiobookChapters {
    /// Fügt die Kapitel aller Teile zu einer durchgehenden Liste zusammen.
    /// Ein Teil ohne eigene Kapitel wird ein Kapitel mit dem Teiltitel.
    /// Lücken und Überlappungen werden geglättet: jedes Kapitel endet am
    /// Beginn des nächsten, das letzte am Ende des Hörbuchs.
    public static func merge(files: [AudiobookFileInfo], tracks: [LibraryAudioTrack]) -> [AudiobookChapter] {
        var out: [AudiobookChapter] = []
        var offset = 0.0
        for (i, track) in tracks.enumerated() {
            let file = i < files.count ? files[i] : AudiobookFileInfo()
            let length = track.durationMs ?? file.durationMs ?? file.chapters.map(\.endMs).max() ?? 0
            let own = file.chapters.filter { $0.startMs.isFinite && $0.startMs >= 0 && (length <= 0 || $0.startMs < length) }
                .sorted { $0.startMs < $1.startMs }
            if own.isEmpty {
                out.append(AudiobookChapter(title: cleanTitle(track.title, fallback: "Teil \(i + 1)"), startMs: offset, durationMs: length))
            } else {
                for (n, c) in own.enumerated() {
                    // Erstes Kapitel beginnt am Teilanfang, damit kein Vorspann ohne Kapitel bleibt.
                    let start = n == 0 ? 0 : c.startMs
                    out.append(AudiobookChapter(title: cleanTitle(c.title, fallback: "Kapitel \(out.count + 1)"),
                                                startMs: offset + start, durationMs: c.durationMs))
                }
            }
            offset += length
        }
        return normalized(out, totalMs: offset)
    }

    /// Kapitel sortieren, Dauer aus dem Abstand zum nächsten Beginn ableiten.
    public static func normalized(_ chapters: [AudiobookChapter], totalMs: Double) -> [AudiobookChapter] {
        let sorted = chapters.sorted { $0.startMs < $1.startMs }
        var out: [AudiobookChapter] = []
        for (i, c) in sorted.enumerated() {
            let end = i + 1 < sorted.count ? sorted[i + 1].startMs : (totalMs > 0 ? totalMs : c.endMs)
            let duration = max(0, end - c.startMs)
            // Doppelte Startpunkte (Kapitel ohne Länge) zusammenfassen.
            if duration <= 0, i + 1 < sorted.count { continue }
            out.append(AudiobookChapter(title: c.title, startMs: c.startMs, durationMs: duration))
        }
        return out
    }

    /// Kapitel, in dem `ms` liegt (letztes mit Beginn ≤ ms).
    public static func index(at ms: Double, in chapters: [AudiobookChapter]) -> Int {
        guard !chapters.isEmpty else { return 0 }
        var lo = 0, hi = chapters.count - 1, found = 0
        while lo <= hi {
            let mid = (lo + hi) / 2
            if chapters[mid].startMs <= ms + 0.5 { found = mid; lo = mid + 1 } else { hi = mid - 1 }
        }
        return found
    }

    /// Anteil 0…1 innerhalb des Kapitels.
    public static func fraction(at ms: Double, in chapter: AudiobookChapter) -> Double {
        guard chapter.durationMs > 0 else { return 0 }
        return min(1, max(0, (ms - chapter.startMs) / chapter.durationMs))
    }

    /// Ziel für „Kapitel zurück“: in den ersten 3 s an den Beginn des
    /// vorigen Kapitels, sonst an den Beginn des aktuellen (wie gewohnt bei Playern).
    public static func previousTarget(at ms: Double, in chapters: [AudiobookChapter]) -> Double {
        guard !chapters.isEmpty else { return 0 }
        let i = index(at: ms, in: chapters)
        if ms - chapters[i].startMs > 3000 || i == 0 { return chapters[i].startMs }
        return chapters[i - 1].startMs
    }

    /// Ziel für „Kapitel vor“; `nil` im letzten Kapitel.
    public static func nextTarget(at ms: Double, in chapters: [AudiobookChapter]) -> Double? {
        let i = index(at: ms, in: chapters)
        return i + 1 < chapters.count ? chapters[i + 1].startMs : nil
    }

    /// Geschätzte Seite: linear über die Hörzeit. Hörbücher haben keine
    /// Seiten; die Zahl dient nur als vertraute Orientierung und wird in der
    /// Oberfläche als „etwa“ ausgewiesen.
    public static func estimatedPage(at ms: Double, totalMs: Double, pages: Int) -> Int {
        guard pages > 0, totalMs > 0 else { return 0 }
        return min(pages, max(1, Int((ms / totalMs * Double(pages)).rounded(.down)) + 1))
    }

    /// Seitenzahl des Buchs: Angabe aus den Metadaten, sonst aus der Textlänge
    /// (etwa 1 800 Zeichen je Taschenbuchseite), sonst aus der Hördauer
    /// (etwa 1,7 Minuten je Seite bei normalem Sprechtempo).
    public static func pageCount(known: Int?, textUTF16: Int?, totalMs: Double) -> (pages: Int, estimated: Bool)? {
        if let known, known > 0 { return (known, false) }
        if let textUTF16, textUTF16 > 2000 { return (max(1, Int((Double(textUTF16) / 1800).rounded())), true) }
        if totalMs > 60_000 { return (max(1, Int((totalMs / 102_000).rounded())), true) }
        return nil
    }

    /// Kapiteltitel aufräumen: Dateiendung, führende Tracknummern.
    public static func cleanTitle(_ raw: String, fallback: String) -> String {
        var t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        for ext in [".mp3", ".m4a", ".m4b", ".aac"] where t.lowercased().hasSuffix(ext) { t.removeLast(ext.count) }
        t = t.replacingOccurrences(of: #"^\d{1,3}[\s_]*[-_]?[\s_]+"#, with: "", options: .regularExpression)
        t = t.replacingOccurrences(of: "_", with: " ").trimmingCharacters(in: .whitespaces)
        return t.isEmpty ? fallback : t
    }
}

extension AudiobookInfo {
    /// Fasst die Angaben mehrerer Teile zusammen: erster gefüllter Wert gewinnt,
    /// Personenlisten ohne Doppel. Bytes werden summiert.
    public static func combine(_ parts: [AudiobookInfo]) -> AudiobookInfo {
        var out = AudiobookInfo()
        func first(_ key: KeyPath<AudiobookInfo, String?>) -> String? { parts.lazy.compactMap { $0[keyPath: key] }.first { !$0.isEmpty } }
        out.title = first(\.title); out.subtitle = first(\.subtitle); out.album = first(\.album)
        out.series = first(\.series); out.publisher = first(\.publisher); out.releaseDate = first(\.releaseDate)
        out.genre = first(\.genre); out.language = first(\.language); out.summary = first(\.summary)
        out.copyright = first(\.copyright); out.encoder = first(\.encoder); out.format = first(\.format); out.codec = first(\.codec)
        out.authors = unique(parts.flatMap(\.authors)); out.narrators = unique(parts.flatMap(\.narrators))
        out.bitrateKbps = parts.lazy.compactMap(\.bitrateKbps).first
        let bytes = parts.compactMap(\.fileBytes)
        out.fileBytes = bytes.isEmpty ? nil : bytes.reduce(0, +)
        out.hasCover = parts.contains { $0.hasCover }
        return out
    }

    /// Personen trennen („A, B & C“, „A; B“, „A und B“) und Doppel entfernen.
    public static func people(_ raw: String?) -> [String] {
        guard let raw, !raw.isEmpty else { return [] }
        let parts = raw.replacingOccurrences(of: " & ", with: ",").replacingOccurrences(of: " und ", with: ",")
            .replacingOccurrences(of: ";", with: ",").replacingOccurrences(of: "/", with: ",")
            .split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        return unique(parts)
    }

    static func unique(_ list: [String]) -> [String] {
        var seen = Set<String>(); var out: [String] = []
        for s in list where seen.insert(s.lowercased()).inserted { out.append(s) }
        return out
    }

    /// Erscheinungsdatum lesbar: „2019-03-14T…“ → „14.03.2019“, „2019“ bleibt.
    public static func displayDate(_ raw: String?) -> String? {
        guard let raw = raw?.trimmingCharacters(in: .whitespaces), !raw.isEmpty else { return nil }
        let digits = raw.prefix(10)
        let p = digits.split(separator: "-")
        if p.count == 3, p[0].count == 4 { return "\(p[2]).\(p[1]).\(p[0])" }
        if p.count == 2, p[0].count == 4 { return "\(p[1]).\(p[0])" }
        return raw
    }
}
